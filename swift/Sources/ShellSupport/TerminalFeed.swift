import Foundation

/// Snapshot-to-stream reconciler for the terminal view.
///
/// The core owns its emulator and hands out plain-text screen snapshots
/// (`am_screen_text`); the SwiftTerm view owns a second emulator that
/// wants an output *stream*. This type computes the smallest feed that
/// makes the view show the new output without duplicating what it
/// already shows:
///
/// - identical snapshots feed nothing;
/// - an append-only snapshot feeds just the suffix (the hot path:
///   streaming agent output and echoed typing);
/// - a scrolled snapshot feeds the new trailing lines (the overlap
///   between the old tail and the new head is already on screen);
/// - anything else (redraw, reflow after resize, cursor-addressed
///   programs) clears and replays the whole snapshot.
///
/// Newlines are normalized to CRLF: the view interprets a bare LF as
/// line-feed-only, which would stair-step the output.
public enum TerminalFeed {
    /// ANSI reset the view understands: clear screen, home cursor.
    public static let clearScreen = "\u{1B}[2J\u{1B}[H"

    /// Feed text that advances a view showing `old` to also show `new`,
    /// or `nil` when the view is already current.
    public static func delta(old: String, new: String) -> String? {
        if new == old {
            return nil
        }
        if !old.isEmpty, new.hasPrefix(old) {
            let suffix = String(new.dropFirst(old.count))
            return suffix.isEmpty ? nil : normalizeNewlines(suffix)
        }
        if old.isEmpty {
            return normalizeNewlines(new)
        }
        let olds = splitLines(old)
        let news = splitLines(new)
        let overlap = largestOverlap(old: olds, new: news)
        if overlap > 0 {
            let tail = news.dropFirst(overlap)
            // The overlapped head is already on screen; the cursor sits
            // at the end of the previously fed text, so start a new line.
            return "\r\n" + tail.joined(separator: "\r\n")
        }
        return clearScreen + normalizeNewlines(new)
    }

    /// Largest k such that the last k lines of `old` equal the first k
    /// lines of `new`. Quadratic in the worst case; snapshots are a few
    /// hundred short lines, so this stays in the microseconds.
    static func largestOverlap(old: [String], new: [String]) -> Int {
        let maxK = min(old.count, new.count)
        var k = maxK
        while k > 0 {
            if Array(old.suffix(k)) == Array(new.prefix(k)) {
                return k
            }
            k -= 1
        }
        return 0
    }

    static func splitLines(_ s: String) -> [String] {
        s.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
    }

    static func normalizeNewlines(_ s: String) -> String {
        s.replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\n", with: "\r\n")
    }

    /// Split helper kept public for tests: (lines, hadTrailingNewline).
    static func newlines(_ s: String) -> ([String], Bool) {
        (splitLines(s), s.hasSuffix("\n"))
    }
}
