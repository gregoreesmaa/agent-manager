/// Two-dimensional new-session picker model (folder × CLI + yolo).
///
/// Pure Swift with no C ABI references, so it unit-tests without linking
/// the core staticlib (same split as `TerminalFeed`). The core owns the
/// same semantics framework-free (`src/launch.rs`); this mirrors the
/// picker half a native shell renders: the CLI catalog rows (decoded
/// from `am_clis_json`), the folder input + recents (from
/// `am_recent_json`), and the per-run yolo tri-state.
///
/// Split-button contract (see `docs/new-session-picker.md`): the main
/// action repeats the last launch instantly (null CLI/folder through
/// `am_spawn_launch`, which resolves the core's effective default); the
/// picker confirms an explicit folder × CLI + yolo combination.
struct NewSessionPicker {
    /// CLI catalog rows in core order (muse, claude, opencode, codex).
    /// Rows are always listed; `available == false` renders disabled
    /// with an install hint, never hidden.
    struct CliRow: Equatable {
        var id: String
        var program: String
        var path: String?
        var available: Bool
    }

    /// Per-run yolo choice: safe by default, explicit per run, never
    /// auto-written back to the config.
    enum YoloChoice: Hashable {
        case useDefault
        case forceOn
        case forceOff

        /// Cycle for the picker toggle: default → on → off → default.
        mutating func cycle() {
            switch self {
            case .useDefault: self = .forceOn
            case .forceOn: self = .forceOff
            case .forceOff: self = .useDefault
            }
        }

        /// Short label for the picker row (text, never color-only).
        var label: String {
            switch self {
            case .useDefault: "Default"
            case .forceOn: "On (once)"
            case .forceOff: "Off (once)"
            }
        }
    }

    var clis: [CliRow]
    var cliIndex: Int
    var folder: String
    var recents: [String]
    var yolo: YoloChoice = .useDefault

    /// Blank folder means inherit (the historic behavior); anything else
    /// spawns in the typed directory (the shell validates is-dir before
    /// confirming, mirroring the gpui picker's inline refuse-and-fix).
    var effectiveFolder: String? {
        let trimmed = folder.trimmingCharacters(in: .whitespaces)
        return trimmed.isEmpty ? nil : trimmed
    }

    var selectedCli: CliRow? {
        guard !clis.isEmpty else { return nil }
        return clis[cliIndex % clis.count]
    }

    /// One-line spawn preview (`muse in ~/api`), so the launch is
    /// verifiable before it runs. The yolo flag itself rides inside
    /// `am_spawn_launch`; the preview names the combination, not argv.
    var preview: String {
        let cli = selectedCli?.id ?? "muse"
        let where_ = effectiveFolder.map { " in \($0)" } ?? ""
        let yoloTag: String
        switch yolo {
        case .useDefault: yoloTag = ""
        case .forceOn: yoloTag = " + yolo"
        case .forceOff: yoloTag = " (yolo off)"
        }
        return "\(cli)\(where_)\(yoloTag)"
    }

    mutating func stepCli(forward: Bool) {
        guard !clis.isEmpty else { return }
        if forward {
            cliIndex = (cliIndex + 1) % clis.count
        } else {
            cliIndex = (cliIndex + clis.count - 1) % clis.count
        }
    }
}
