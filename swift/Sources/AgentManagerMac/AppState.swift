import Foundation
import ShellSupport

/// Pump state for every live session, on the main thread.
///
/// The roster itself is a snapshot taken at launch (discovery +
/// persisted rows merged inside `am_core_new`); statuses refresh on
/// every pump tick via `am_status`. PTYs live in `ptys` keyed by row
/// id; `fedText` remembers the snapshot already shown in the
/// terminal view so the pump can feed only the delta.
@MainActor
final class AppState: ObservableObject {
    /// Default grid until the terminal view reports its real size.
    static let defaultCols = 80
    static let defaultRows = 24

    @Published private(set) var rows: [SessionRow] = []
    @Published private(set) var statuses: [String: Int] = [:]
    @Published var selection: String?
    @Published var filter = ""
    @Published var pendingError: String?
    @Published var savedFlash = false

    /// Feeds waiting for the terminal view: row id -> (text, sequence).
    /// The sequence lets the view skip what it already fed.
    @Published private(set) var feeds: [String: (text: String, seq: UInt64)] = [:]

    private let core: Core
    private var ptys: [String: Pty] = [:]
    private var fedText: [String: String] = [:]
    private var feedSeq: UInt64 = 0
    private var grid = (cols: defaultCols, rows: defaultRows)
    private var timer: Timer?

    init(core: Core) {
        self.core = core
        reloadRoster()
        if selection == nil {
            selection = rows.first?.id
        }
        timer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) {
            [weak self] _ in
            MainActor.assumeIsolated { self?.pump() }
        }
    }

    deinit { timer?.invalidate() }

    // MARK: - Roster

    var attentionCount: Int {
        statuses.values.filter { $0 == RunStatus.attention.rawValue }.count
    }

    var filteredRows: [SessionRow] {
        let q = filter.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return rows }
        return rows.filter {
            $0.title.lowercased().contains(q)
                || $0.project.lowercased().contains(q)
                || $0.id.lowercased().contains(q)
        }
    }

    func rows(with status: RunStatus) -> [SessionRow] {
        filteredRows.filter { statuses[$0.id] == status.rawValue }
    }

    func status(of row: SessionRow) -> RunStatus {
        RunStatus(rawValue: statuses[row.id] ?? RunStatus.idle.rawValue) ?? .idle
    }

    private func reloadRoster() {
        var loaded: [SessionRow] = []
        for i in 0 ..< core.sessionCount {
            if let row = core.sessionRow(i) {
                loaded.append(row)
                statuses[row.id] = Int(core.status(i))
            }
        }
        rows = loaded
    }

    // MARK: - Spawn / converse / resize

    var canSpawn: Bool {
        guard let id = selection else { return false }
        return ptys[id] == nil && rows.contains(where: { $0.id == id })
    }

    func spawnSelected() {
        guard let id = selection, ptys[id] == nil else { return }
        do {
            ptys[id] = try core.spawn(cols: grid.cols, rows: grid.rows)
            fedText[id] = ""
        } catch {
            pendingError = error.localizedDescription
        }
    }

    func hasLivePty(_ id: String) -> Bool { ptys[id] != nil }

    func sendToPty(rowId: String, bytes: [UInt8]) {
        do {
            try ptys[rowId]?.write(bytes)
        } catch {
            pendingError = error.localizedDescription
        }
    }

    func terminalResized(rowId _: String, cols: Int, rows: Int) {
        grid = (cols, rows)
        for pty in ptys.values {
            pty.resize(cols: cols, rows: rows)
        }
    }

    /// Feed text the terminal view has not shown yet, if any.
    func takeFeed(for rowId: String, after seq: UInt64) -> String? {
        guard let feed = feeds[rowId], feed.seq != seq else { return nil }
        return feed.text
    }

    func feedSeq(for rowId: String) -> UInt64 { feeds[rowId]?.seq ?? 0 }

    // MARK: - Persistence

    func save() {
        do {
            try core.save()
            savedFlash = true
        } catch {
            pendingError = error.localizedDescription
        }
    }

    // MARK: - Pump

    private func pump() {
        for (id, pty) in ptys {
            guard pty.pump() else { continue }
            // Color-preserving render of the same screen: the styled spans
            // re-emitted as SGR when they decode, else the plain snapshot.
            // The reconciler below works on either form, so both stay
            // duplication-free and resize-safe; `fedText` stores whichever
            // form was shown last.
            let current: String
            if let spans = pty.spansJson(),
               let styled = AnsiFeed.render(json: spans)
            {
                current = styled
            } else if let snapshot = pty.screenText() {
                current = snapshot
            } else {
                continue
            }
            let shown = fedText[id, default: ""]
            if let feed = TerminalFeed.delta(old: shown, new: current),
               !feed.isEmpty
            {
                feedSeq += 1
                feeds[id] = (feed, feedSeq)
            }
            fedText[id] = current
        }
        for i in 0 ..< rows.count {
            statuses[rows[i].id] = Int(core.status(i))
        }
    }
}
