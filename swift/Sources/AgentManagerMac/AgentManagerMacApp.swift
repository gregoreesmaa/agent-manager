import SwiftUI

/// macOS shell over the core staticlib (issue #62).
///
/// The roster, spawn, pump, write, resize, and screen-text calls all go
/// through the C ABI in `CoreBridge.swift`; rendering is SwiftUI plus a
/// SwiftTerm terminal view. Entry is via `main.swift` (which also hosts
/// the `--smoke` runtime check).
struct AgentManagerMacApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate
    @StateObject private var appState: AppState
    @AppStorage("appearance") private var appearance = "system"

    init() {
        guard let core = Core() else {
            fatalError("am_core_new returned NULL (allocation failure)")
        }
        let state = AppState(core: core)
        _appState = StateObject(wrappedValue: state)
        delegate.state = state
    }

    var body: some Scene {
        WindowGroup {
            ContentView(state: appState)
                .preferredColorScheme(scheme)
                .frame(minWidth: 720, minHeight: 460)
        }
        // No title bar and no toolbar: the window keeps only the
        // floating traffic lights, so the sidebar and terminal own the
        // full height.
        .windowStyle(.hiddenTitleBar)
        .commands {
            CommandGroup(after: .saveItem) {
                Button("Save Core Config", action: appState.save)
                    .keyboardShortcut("s", modifiers: .command)
            }
        }
    }

    private var scheme: ColorScheme? {
        switch appearance {
        case "dark": .dark
        case "light": .light
        default: nil
        }
    }
}

/// App delegate: activation fix + config persistence on quit.
///
/// This target is a bare SwiftPM executable (no app bundle, no
/// Info.plist), so AppKit never grants it regular activation: the
/// process stays invisible to the window server and the WindowGroup
/// window never appears (issue #67). Becoming a regular app and
/// activating on launch is what makes the window show.
final class AppDelegate: NSObject, NSApplicationDelegate {
    var state: AppState?

    private var windowObserver: NSObjectProtocol?

    func applicationDidFinishLaunching(_: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        WindowPaint.set(darkMode: WindowPaint.current())
        // The WindowGroup window may not exist yet at launch, so paint
        // on every main-window change too.
        windowObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didBecomeMainNotification,
            object: nil, queue: .main
        ) { _ in WindowPaint.set(darkMode: WindowPaint.current()) }
    }

    func applicationWillTerminate(_: Notification) {
        // Best effort on the way out; errors have nowhere to show.
        MainActor.assumeIsolated {
            self.state?.save()
        }
    }
}

/// Window background = terminal background (pure black in Dark Mode,
/// white in Light Mode), so any gap around the terminal reads as
/// terminal instead of window gray.
enum WindowPaint {
    static func set(darkMode: Bool) {
        for window in NSApp.windows {
            window.backgroundColor = darkMode ? .black : .white
        }
    }

    /// Resolve the theme the same way ContentView does: a forced
    /// appearance wins, otherwise the system appearance.
    static func current() -> Bool {
        switch UserDefaults.standard.string(forKey: "appearance") {
        case "dark": return true
        case "light": return false
        default:
            return NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        }
    }
}
