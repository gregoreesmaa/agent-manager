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

/// Persists the core config when the app quits.
final class AppDelegate: NSObject, NSApplicationDelegate {
    var state: AppState?

    func applicationWillTerminate(_: Notification) {
        // Best effort on the way out; errors have nowhere to show.
        MainActor.assumeIsolated {
            self.state?.save()
        }
    }
}
