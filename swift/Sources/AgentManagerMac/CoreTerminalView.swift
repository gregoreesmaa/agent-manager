import AppKit
import SwiftTerm
import SwiftUI

/// The session terminal: a SwiftTerm `TerminalView` (native selection,
/// copy/paste, scrollback, and Cmd-F find) driven by the core.
///
/// Output flows core PTY -> `am_pump`/`am_screen_text`+`am_spans_json` ->
/// SGR snapshot delta -> `view.feed(text:)`; keystrokes flow view ->
/// `send` delegate ->
/// `am_write`; window resizes flow view -> `sizeChanged` -> `am_resize`.
/// The view follows the theme (text colors resolved against an explicit
/// light/dark appearance) and owns all of the terminal grid; no terminal
/// grid is drawn here.
struct CoreTerminalView: NSViewRepresentable {
    @ObservedObject var state: AppState
    var rowId: String
    var darkMode: Bool

    func makeCoordinator() -> Coordinator {
        Coordinator(state: state, rowId: rowId)
    }

    func makeNSView(context: Context) -> TerminalView {
        let font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)
        let view = TerminalView(frame: .zero, font: font)
        applyThemeColors(to: view, darkMode: darkMode)
        view.terminalDelegate = context.coordinator
        context.coordinator.lastDarkMode = darkMode
        // Become key so keystrokes reach the child right away.
        DispatchQueue.main.async {
            view.window?.makeFirstResponder(view)
        }
        return view
    }

    func updateNSView(_ view: TerminalView, context: Context) {
        context.coordinator.drainFeed(into: view)
        if context.coordinator.lastDarkMode != darkMode {
            context.coordinator.lastDarkMode = darkMode
            applyThemeColors(to: view, darkMode: darkMode)
        }
    }

    /// Terminal colors as a pure function of the theme prop — never of
    /// ambient appearance. Snapshotting ambient colors can catch the
    /// pre-flip mode (e.g. a fresh view still on the system appearance
    /// under a forced theme), and the change gate above would then lock
    /// those stale colors in, leaving the terminal one mode behind the
    /// sidebar forever. Background is pure terminal black/white, not
    /// the dark-gray system fill.
    private func applyThemeColors(to view: TerminalView, darkMode: Bool) {
        var fg: NSColor?
        if let appearance = NSAppearance(named: darkMode ? .darkAqua : .aqua) {
            appearance.performAsCurrentDrawingAppearance {
                fg = NSColor.textColor.usingColorSpace(.sRGB)
            }
        }
        view.nativeForegroundColor = fg ?? .textColor
        view.nativeBackgroundColor = darkMode ? .black : .white
    }

    @MainActor
    final class Coordinator: NSObject, @MainActor TerminalViewDelegate {
        private let state: AppState
        private let rowId: String
        fileprivate var lastFedSeq: UInt64 = 0
        fileprivate var lastDarkMode = false

        init(state: AppState, rowId: String) {
            self.state = state
            self.rowId = rowId
        }

        /// Feed anything the pump produced since the last drain.
        func drainFeed(into view: TerminalView) {
            let seq = state.feedSeq(for: rowId)
            guard seq != lastFedSeq,
                  let text = state.takeFeed(for: rowId, after: lastFedSeq)
            else { return }
            lastFedSeq = seq
            view.feed(text: text)
        }

        // MARK: - TerminalViewDelegate

        /// Keystrokes and pastes, already key-encoded: straight to the child.
        func send(source _: TerminalView, data: ArraySlice<UInt8>) {
            state.sendToPty(rowId: rowId, bytes: Array(data))
        }

        /// The view recomputes its grid from the window frame; keep the
        /// core PTY and its emulator the same size (core null-op safe).
        func sizeChanged(source _: TerminalView, newCols: Int, newRows: Int) {
            state.terminalResized(rowId: rowId, cols: newCols, rows: newRows)
        }

        func setTerminalTitle(source _: TerminalView, title _: String) {}
        func hostCurrentDirectoryUpdate(source _: TerminalView, directory _: String?) {}
        func scrolled(source _: TerminalView, position _: Double) {}

        /// The view's own damage tracker; the shell repaints from the
        /// pump instead, so this is a no-op.
        func rangeChanged(source _: TerminalView, startY _: Int, endY _: Int) {}
    }
}
