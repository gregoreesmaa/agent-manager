import XCTest

@testable import ShellSupport

final class AnsiFeedTests: XCTestCase {
    func testRedSpanRoundTrip() {
        // The brief's falsifiable: `printf '\e[31mred\e[0m\n'` leaves a red
        // span (palette 205,0,0) in the core snapshot; it must come back
        // out as red SGR for the view.
        let json = #"[[{"text":"red","fg":[205,0,0],"bg":null,"bold":false,"italic":false,"underline":false}]]"#
        XCTAssertEqual(
            AnsiFeed.render(json: json),
            "\u{1B}[0;38;2;205;0;0mred"
        )
    }

    func testDefaultSpanEmitsReset() {
        let json = #"[[{"text":"plain","fg":null,"bg":null,"bold":false,"italic":false,"underline":false}]]"#
        XCTAssertEqual(AnsiFeed.render(json: json), "\u{1B}[0mplain")
    }

    func testBoldUnderlineBackgroundCombineAndRowsJoinWithLF() {
        let json = #"[[{"text":"a","fg":null,"bg":[0,0,238],"bold":true,"italic":false,"underline":true}],[{"text":"b","fg":null,"bg":null,"bold":false,"italic":false,"underline":false}]]"#
        XCTAssertEqual(
            AnsiFeed.render(json: json),
            "\u{1B}[0;1;4;48;2;0;0;238ma\n\u{1B}[0mb"
        )
    }

    func testBadJsonFallsBackToNil() {
        // Nil tells the pump to feed the plain-text snapshot instead.
        XCTAssertNil(AnsiFeed.render(json: "not json"))
        XCTAssertNil(AnsiFeed.render(json: #"{"text": 1}"#))
        XCTAssertNil(AnsiFeed.render(json: ""))
    }

    func testStyledAppendFeedsSuffixOnly() {
        // Colors must not break the hot path: appended output still feeds
        // just the suffix, carrying its own style state.
        XCTAssertEqual(
            TerminalFeed.delta(
                old: "\u{1B}[0mhello",
                new: "\u{1B}[0mhello\u{1B}[0;38;2;205;0;0m red"
            ),
            "\u{1B}[0;38;2;205;0;0m red"
        )
    }

    func testStyledFirstSnapshotFeedsWholeScreen() {
        XCTAssertEqual(
            TerminalFeed.delta(old: "", new: "\u{1B}[0;38;2;205;0;0mred"),
            "\u{1B}[0;38;2;205;0;0mred"
        )
    }

    func testStyledColorOnlyChangeRedraws() {
        // Same text, new color: the worn frame must not be reused.
        let feed = TerminalFeed.delta(
            old: "\u{1B}[0mhello",
            new: "\u{1B}[0;38;2;205;0;0mhello"
        )
        XCTAssertTrue(feed?.hasPrefix(TerminalFeed.clearScreen) ?? false)
        XCTAssertTrue(feed?.contains("38;2;205;0;0") ?? false)
    }
}
