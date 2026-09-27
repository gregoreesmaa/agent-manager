import XCTest

@testable import ShellSupport

final class TerminalFeedTests: XCTestCase {
    func testIdenticalSnapshotsFeedNothing() {
        XCTAssertNil(TerminalFeed.delta(old: "a\nb", new: "a\nb"))
        XCTAssertNil(TerminalFeed.delta(old: "", new: ""))
    }

    func testAppendedOutputFeedsSuffixOnly() {
        // Streaming agent output: the hot path feeds just the new tail.
        XCTAssertEqual(
            TerminalFeed.delta(old: "hello", new: "hello world"),
            " world"
        )
    }

    func testAppendedLinesNormalizeToCRLF() {
        // Bare LFs would stair-step in the view; they go out as CRLF.
        XCTAssertEqual(
            TerminalFeed.delta(old: "a", new: "a\nb\n"),
            "\r\nb\r\n"
        )
    }

    func testFirstSnapshotFeedsWholeScreen() {
        XCTAssertEqual(
            TerminalFeed.delta(old: "", new: "ready\n$ "),
            "ready\r\n$ "
        )
    }

    func testScrolledSnapshotFeedsNewTrailingLines() {
        // Screen scrolled by one: the shared "b" line is already shown.
        XCTAssertEqual(
            TerminalFeed.delta(old: "a\nb", new: "b\nc"),
            "\r\nc"
        )
        XCTAssertEqual(
            TerminalFeed.delta(old: "a\nb\nc", new: "c\nd\ne"),
            "\r\nd\r\ne"
        )
    }

    func testRedrawClearsAndReplays() {
        // No append/overlap relation: clear first, then the snapshot.
        let feed = TerminalFeed.delta(old: "menu: [x]", new: "other screen")
        XCTAssertEqual(feed, TerminalFeed.clearScreen + "other screen")
    }

    func testResizeReflowFallsBackToRedraw() {
        // Rewrapped lines share no clean overlap; the view replays.
        let feed = TerminalFeed.delta(old: "a very long line here", new: "a very\nlong line\nhere")
        XCTAssertTrue(feed?.hasPrefix(TerminalFeed.clearScreen) ?? false)
    }

    func testLargestOverlap() {
        XCTAssertEqual(
            TerminalFeed.largestOverlap(old: ["a", "b"], new: ["b", "c"]),
            1
        )
        XCTAssertEqual(
            TerminalFeed.largestOverlap(old: ["a"], new: ["b"]),
            0
        )
        XCTAssertEqual(
            TerminalFeed.largestOverlap(old: [], new: ["b"]),
            0
        )
    }
}
