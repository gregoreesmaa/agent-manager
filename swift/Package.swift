// swift-tools-version: 5.9
// AgentManagerMac: native macOS shell over the core staticlib C ABI
// (issue #62). The Rust core builds separately (`cargo build --lib`);
// this manifest only tells the linker where that staticlib lives.
import Foundation
import PackageDescription

let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let targetDir = packageDir
    .deletingLastPathComponent()
    .appendingPathComponent("target")

func libDir(_ config: String) -> String {
    targetDir.appendingPathComponent(config).path
}

let package = Package(
    name: "AgentManagerMac",
    platforms: [.macOS(.v13)],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm.git", from: "1.20.0"),
    ],
    targets: [
        // Pure-Swift helpers with no C ABI references (unit-testable
        // without linking the core staticlib).
        .target(
            name: "ShellSupport",
            path: "Sources/ShellSupport"
        ),
        .executableTarget(
            name: "AgentManagerMac",
            dependencies: [
                "ShellSupport",
                .product(name: "SwiftTerm", package: "SwiftTerm"),
            ],
            path: "Sources/AgentManagerMac",
            linkerSettings: [
                .linkedLibrary("agent_manager"),
                .unsafeFlags([
                    "-L", libDir("debug"),
                    "-L", libDir("release"),
                ]),
            ]
        ),
        .testTarget(
            name: "ShellSupportTests",
            dependencies: ["ShellSupport"],
            path: "Tests/ShellSupportTests"
        ),
    ]
)
