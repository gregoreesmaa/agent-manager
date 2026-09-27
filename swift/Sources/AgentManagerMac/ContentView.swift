import SwiftUI

/// Roster sidebar + session terminal.
///
/// - Roster: core rows grouped by urgency (needs-input first), with a
///   native search field filtering title/project/id.
/// - Spawn: toolbar button (or the empty-state button) starts the
///   selected row's child via `am_spawn`; typing in the terminal
///   converses through `am_write`.
/// - History: every row shows its project, harness, and last-active age;
///   the rows themselves come from discovery + the persisted store, so
///   they survive relaunches.
/// - Theme: System/Dark/Light picker persisted in UserDefaults.
/// - Persistence: Save writes the core config; it also runs on quit.
struct ContentView: View {
    @ObservedObject var state: AppState
    @AppStorage("appearance") private var appearance = "system"
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        NavigationSplitView {
            List(selection: $state.selection) {
                if state.filteredRows.isEmpty {
                    Text("No sessions match.").foregroundStyle(.secondary)
                }
                ForEach(statusSections, id: \.status) { section in
                    if !section.rows.isEmpty {
                        Section("\(section.title) (\(section.rows.count))") {
                            ForEach(section.rows) { row in
                                rowLabel(row)
                                    .tag(row.id)
                                    .badge(state.hasLivePty(row.id) ? "live" : nil)
                            }
                        }
                    }
                }
            }
            .searchable(text: $state.filter, prompt: "Filter sessions")
            .navigationTitle(title)
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("Spawn", action: state.spawnSelected)
                        .keyboardShortcut("n", modifiers: .command)
                        .disabled(!state.canSpawn)
                        .help("Start the selected session (am_spawn)")
                }
                ToolbarItem {
                    Button("Save", action: state.save)
                        .keyboardShortcut("s", modifiers: .command)
                        .help("Persist the core config (am_core_save)")
                }
                ToolbarItem {
                    Picker("Theme", selection: $appearance) {
                        Text("System").tag("system")
                        Text("Dark").tag("dark")
                        Text("Light").tag("light")
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 200)
                }
            }
        } detail: {
            detailView
        }
        .alert("Session error", isPresented: errorPresented) {
            Button("OK", role: .cancel) { state.pendingError = nil }
        } message: {
            Text(state.pendingError ?? "")
        }
    }

    // MARK: - Sidebar

    private var title: String {
        let n = state.attentionCount
        return n == 0 ? "Sessions" : "Sessions (\(n) need input)"
    }

    private struct StatusSection {
        var status: Int
        var title: String
        var rows: [SessionRow]
    }

    private var statusSections: [StatusSection] {
        [
            StatusSection(
                status: RunStatus.attention.rawValue, title: "Needs input",
                rows: state.rows(with: .attention)
            ),
            StatusSection(
                status: RunStatus.working.rawValue, title: "Working",
                rows: state.rows(with: .working)
            ),
            StatusSection(
                status: RunStatus.idle.rawValue, title: "Idle",
                rows: state.rows(with: .idle)
            ),
        ]
    }

    private func rowLabel(_ row: SessionRow) -> some View {
        HStack {
            statusDot(state.status(of: row))
            VStack(alignment: .leading) {
                Text(row.title).lineLimit(1)
                Text("\(row.project) · \(row.harness) · \(age(row.lastActive))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            if row.linkCount > 0 {
                Text("\(row.linkCount) link\(row.linkCount == 1 ? "" : "s")")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func statusDot(_ status: RunStatus) -> some View {
        let color: Color = switch status {
        case .attention: .red
        case .working: .green
        case .idle: .gray
        }
        return Circle().fill(color).frame(width: 8, height: 8)
    }

    private func age(_ unixSeconds: Int64) -> String {
        let delta = max(0, Int(Date.now.timeIntervalSince1970) - Int(unixSeconds))
        if delta < 60 { return "just now" }
        if delta < 3600 { return "\(delta / 60)m ago" }
        if delta < 86400 { return "\(delta / 3600)h ago" }
        return "\(delta / 86400)d ago"
    }

    // MARK: - Detail

    @ViewBuilder
    private var detailView: some View {
        if let id = state.selection,
           let row = state.rows.first(where: { $0.id == id })
        {
            if state.hasLivePty(id) {
                CoreTerminalView(
                    state: state, rowId: id,
                    darkMode: colorScheme == .dark
                )
            } else {
                VStack(spacing: 12) {
                    Text(row.title).font(.title2)
                    Text("\(row.project) · \(row.harness)")
                        .foregroundStyle(.secondary)
                    Button("Spawn session", action: state.spawnSelected)
                        .keyboardShortcut(.defaultAction)
                        .buttonStyle(.borderedProminent)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        } else if state.rows.isEmpty {
            VStack(spacing: 12) {
                Text("No sessions yet").font(.title2)
                Text("Spawned sessions appear here; history is restored on launch.")
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            Text("Select a session").foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private var errorPresented: Binding<Bool> {
        Binding(
            get: { state.pendingError != nil },
            set: { if !$0 { state.pendingError = nil } }
        )
    }
}
