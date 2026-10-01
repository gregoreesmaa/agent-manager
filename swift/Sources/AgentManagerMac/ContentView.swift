import ShellSupport
import SwiftUI

/// Roster sidebar + session terminal.
///
/// - Roster: core rows grouped by urgency (needs-input first), with an
///   inline sidebar filter field matching title/project/id.
/// - Spawn: the sidebar New Session button (or the detail Spawn
///   button) starts the selected row's child via `am_spawn`; with no
///   selection New Session takes the first visible row. Typing in
///   the terminal converses through `am_write`.
/// - History: every row shows its project, harness, and last-active age;
///   the rows themselves come from discovery + the persisted store, so
///   they survive relaunches.
/// - Theme: follows the system appearance (no manual override).
/// - Persistence: Save writes the core config; it also runs on quit.
struct ContentView: View {
    @ObservedObject var state: AppState
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        NavigationSplitView {
            VStack(spacing: 0) {
                // Split-button 2D launch: the main action repeats the
                // last launch instantly; the menu opens the full picker
                // (folder × CLI + yolo) or repeats explicitly.
                Menu {
                    Button("Repeat last session", action: state.repeatLastSession)
                    Button("Choose folder, CLI, options…") { state.pickerOpen = true }
                        .keyboardShortcut("n", modifiers: [.command, .shift])
                } label: {
                    Label("New Session", systemImage: "plus")
                }
                .menuStyle(.button)
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("n", modifiers: .command)
                .help("Repeat the last session, or pick folder × CLI + yolo")
                .padding(8)
                Divider()
                List(selection: $state.selection) {
                    Text(title)
                        .font(.headline)
                    TextField("Filter sessions", text: $state.filter)
                        .textFieldStyle(.roundedBorder)
                    if state.filteredRows.isEmpty {
                        Text("No sessions match.").foregroundStyle(.secondary)
                    }
                    ForEach(statusSections, id: \.status) { section in
                        if !section.rows.isEmpty {
                            Section {
                                ForEach(section.rows) { row in
                                    rowLabel(row)
                                        .tag(row.id)
                                        .badge(state.hasLivePty(row.id) ? "live" : nil)
                                }
                            } header: {
                                HStack {
                                    Text(section.title)
                                    Spacer()
                                    Text("\(section.rows.count)")
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
                .listStyle(.sidebar)
                Divider()
                sidebarFooter
            }
        } detail: {
            // SwiftUI counts the toolbar height as detail safe area,
            // leaving a toolbar-tall dead gap above the terminal: ignore
            // only the top container inset so the terminal starts at the
            // window edge. Keyboard safe area is untouched.
            detailView.ignoresSafeArea(.container, edges: .top)
        }
        // Repaint the window background when the system appearance
        // changes mid-run.
        .onChange(of: colorScheme) { _, _ in
            WindowPaint.sync()
        }
        .alert("Session error", isPresented: errorPresented) {
            Button("OK", role: .cancel) { state.pendingError = nil }
        } message: {
            Text(state.pendingError ?? "")
        }
        .sheet(isPresented: $state.pickerOpen) {
            NewSessionSheet(state: state, isPresented: $state.pickerOpen)
        }
    }

    // MARK: - Sidebar

    /// Save lives in the sidebar footer (Cmd-S, additionally
    /// wired app-wide in `AgentManagerMacApp.commands`); New Session
    /// sits at the sidebar top (Cmd-N, likewise app-wide). The shell
    /// follows the system appearance: no manual theme override.
    private var sidebarFooter: some View {
        Button("Save", action: state.save)
            .keyboardShortcut("s", modifiers: .command)
            .help("Persist the core config (am_core_save)")
            .padding(8)
    }

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
        {            if state.hasLivePty(id) {
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
        } else if let id = state.selection, state.hasLivePty(id) {
            // Fresh 2D-launch PTY with no roster row yet (repeat-last or
            // picker spawn): show its terminal directly instead of the
            // "Select a session" placeholder.
            CoreTerminalView(
                state: state, rowId: id,
                darkMode: colorScheme == .dark
            )
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

/// 2D new-session sheet: working folder × agent CLI + one-shot yolo.
///
/// - Folder: a text field (blank = inherit) plus the persisted recents
///   for one-click refill. A missing folder refuses inline and stays
///   open for a fix — the sheet never spawns into nothing.
/// - CLI: a picker over the autodetected catalog; missing CLIs render
///   disabled with an install hint, never hidden.
/// - Yolo: a tri-state toggle (default / on once / off once), safe by
///   default; the footer previews the exact combination before Spawn.
private struct NewSessionSheet: View {
    @ObservedObject var state: AppState
    @Binding var isPresented: Bool

    @State private var clis: [CliRow] = []
    @State private var cliId = "muse"
    @State private var folder = ""
    @State private var yolo = NewSessionPicker.YoloChoice.useDefault
    @State private var folderError: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Start a new run").font(.title2)
            // Folder axis.
            Text("Where should it work?").font(.headline)
            TextField("Blank = current folder", text: $folder)
                .textFieldStyle(.roundedBorder)
            if !recents.isEmpty {
                Picker("Recent", selection: $folder) {
                    Text("Type a folder…").tag("")
                    ForEach(recents, id: \.self) { recent in
                        Text(recent).tag(recent)
                    }
                }
                .pickerStyle(.menu)
            }
            if let folderError {
                Text(folderError).foregroundStyle(.red).font(.caption)
            }
            // CLI axis.
            Text("Who should do it?").font(.headline)
            Picker("Agent CLI", selection: $cliId) {
                ForEach(clis, id: \.id) { cli in
                    Text(cliLabel(cli)).tag(cli.id)
                        .disabled(!cli.available)
                }
            }
            .pickerStyle(.radioGroup)
            // Yolo tri-state (safe default; per-run only).
            Text("Permission mode").font(.headline)
            Picker("Yolo", selection: $yolo) {
                Text("Default").tag(NewSessionPicker.YoloChoice.useDefault)
                Text("On (once)").tag(NewSessionPicker.YoloChoice.forceOn)
                Text("Off (once)").tag(NewSessionPicker.YoloChoice.forceOff)
            }
            .pickerStyle(.segmented)
            Text(previewText)
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { isPresented = false }
                Button("Spawn") { spawn() }
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
                    .disabled(clis.isEmpty)
            }
        }
        .padding(20)
        .frame(minWidth: 360)
        .onAppear {
            clis = state.pickerCatalog()
            // Preselect the first available CLI (catalog order); a
            // configured-but-missing default stays listed but never
            // preselected — Spawn would fail for certain.
            if let firstUp = clis.first(where: { $0.available }) {
                if !clis.contains(where: { $0.id == cliId && $0.available }) {
                    cliId = firstUp.id
                }
            } else {
                cliId = clis.first?.id ?? "muse"
            }
            folder = state.pickerRecents().first ?? ""
        }
    }

    private var recents: [String] { state.pickerRecents() }

    private func cliLabel(_ cli: CliRow) -> String {
        cli.available
            ? "\(cli.id) — ready"
            : "\(cli.id) — not installed"
    }

    private var previewText: String {
        let where_ = folder.trimmingCharacters(in: .whitespaces).isEmpty
            ? "" : " in \(folder)"
        let yoloTag: String
        switch yolo {
        case .useDefault: yoloTag = ""
        case .forceOn: yoloTag = " + yolo"
        case .forceOff: yoloTag = " (yolo off)"
        }
        return "runs: \(cliId)\(where_)\(yoloTag)"
    }

    private func spawn() {
        let trimmed = folder.trimmingCharacters(in: .whitespaces)
        let folderOrNil = trimmed.isEmpty ? nil : trimmed
        if let dir = folderOrNil {
            var isDir: ObjCBool = false
            let exists = FileManager.default.fileExists(
                atPath: dir, isDirectory: &isDir)
            if !exists || !isDir.boolValue {
                folderError = "No such folder: \(dir)"
                return
            }
        }
        let yoloArg: Int32
        switch yolo {
        case .useDefault: yoloArg = 0
        case .forceOn: yoloArg = 1
        case .forceOff: yoloArg = -1
        }
        state.confirmPicker(cli: cliId, folder: folderOrNil, yolo: yoloArg)
        isPresented = false
    }
}
