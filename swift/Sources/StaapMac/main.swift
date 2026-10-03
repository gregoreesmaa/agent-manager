import SwiftUI

// Entry point (instead of @main) so `--smoke` can run the headless
// runtime check without starting the UI.
if CommandLine.arguments.contains("--smoke") {
    exit(Smoke.run() ? 0 : 1)
}

StaapMacApp.main()
