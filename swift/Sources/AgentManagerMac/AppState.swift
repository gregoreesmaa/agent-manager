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
    /// 2D-launch picker sheet visibility (set by the menu, the caret,
    /// or Cmd-Shift-N; the sheet resets it on dismiss).
    @Published var pickerOpen = false

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

    func spawnSelected() {
        guard let id = selection, ptys[id] == nil else { return }
        do {
            ptys[id] = try core.spawn(cols: grid.cols, rows: grid.rows)
            fedText[id] = ""
        } catch {
            pendingError = error.localizedDescription
        }
    }

    // MARK: - 2D launch (folder × CLI + yolo)

    /// Split-button main action: repeat the last launch instantly (the
    /// null-CLI/null-folder/zero-yolo form of `am_spawn_launch`, which
    /// resolves the core's effective default: last-used, configured,
    /// autodetected). A failed repeat surfaces in `pendingError`, never
    /// silently.
    func repeatLastSession() {
        do {
            let pty = try core.spawnLaunch(cli: nil, cwd: nil, yolo: 0,
                                           cols: grid.cols, rows: grid.rows)
            attachFreshPty(pty, cli: core.effectiveCli(), folder: nil)
        } catch {
            pendingError = error.localizedDescription
        }
    }

    /// Attach a freshly spawned PTY under a new local id (native shells
    /// mint their own rows: the roster snapshot is launch-time, while
    /// live PTYs key by id like the gpui shell's run map). The row joins
    /// the roster immediately so triage (counts, groups, filter) sees it;
    /// folder/CLI choice is recorded on the row for the detail header.
    private func attachFreshPty(_ pty: Pty, cli: String, folder: String?) {
        let id = "local-\(UInt64.random(in: 0 ... UInt64.max))"
        ptys[id] = pty
        fedText[id] = ""
        let project: String
        if let folder, !folder.isEmpty {
            project = URL(fileURLWithPath: folder).lastPathComponent
        } else {
            project = "local"
        }
        rows.append(SessionRow(
            id: id, title: project, project: project, statusName: "Working",
            harness: cli, lastActive: Int64(Date.now.timeIntervalSince1970),
            cwd: folder, prLinks: nil, relatedLinks: nil
        ))
        statuses[id] = RunStatus.working.rawValue
        selection = id
    }

    /// Confirmed picker launch: explicit folder × CLI + tri-state yolo
    /// (1 = on once, -1 = off once, 0 = config default). Records the
    /// combination via `am_note_launch` so the next repeat replays it.
    func confirmPicker(cli: String, folder: String?, yolo: Int32) {
        do {
            let pty = try core.spawnLaunch(cli: cli, cwd: folder, yolo: yolo,
                                           cols: grid.cols, rows: grid.rows)
            attachFreshPty(pty)
            core.noteLaunch(cli: cli, cwd: folder)
        } catch {
            pendingError = error.localizedDescription
        }
    }

    /// Fresh CLI catalog for the picker sheet (re-read on every open so
    /// the list is never stale).
    func pickerCatalog() -> [CliRow] { core.cliCatalog() }

    /// Folder recents for the picker sheet (MRU-first).
    func pickerRecents() -> [String] { core.recentFolders() }

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
