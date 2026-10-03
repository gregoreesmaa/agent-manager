import Foundation

/// Color-preserving renderer for the core's styled-span snapshots.
///
/// Thin wrapper over the shared core renderer (`am_ansi_render`, via
/// `Core.ansiRender`): one rule for every shell. The pure-Swift
/// implementation below stays for unit tests without linking the core
/// staticlib; production calls the core.
///
/// The core emulator owns SGR state and hands out spans rows of
/// same-style spans; the render re-emits those spans as an ANSI/SGR
/// stream the SwiftTerm view feeds on directly, so program colors
/// survive the pump. Every span carries a complete SGR sequence (reset
/// plus attributes), so any fragment of a render is self-contained.
public enum AnsiFeed {
    /// One styled run inside a snapshot row, matching the core's
    /// `am_spans_json` shape. Style keys default when missing so a
    /// skewed span degrades to the terminal default instead of failing
    /// the whole snapshot (the pump then still shows plain text).
    public struct Span: Decodable {
        public var text: String
        public var fg: [Int]?
        public var bg: [Int]?
        public var bold: Bool
        public var italic: Bool
        public var underline: Bool

        enum CodingKeys: String, CodingKey {
            case text
            case fg
            case bg
            case bold
            case italic
            case underline
        }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            text = try c.decode(String.self, forKey: .text)
            fg = try c.decodeIfPresent([Int].self, forKey: .fg)
            bg = try c.decodeIfPresent([Int].self, forKey: .bg)
            bold = try c.decodeIfPresent(Bool.self, forKey: .bold) ?? false
            italic = try c.decodeIfPresent(Bool.self, forKey: .italic) ?? false
            underline = try c.decodeIfPresent(Bool.self, forKey: .underline) ?? false
        }
    }

    /// Render decoded snapshot rows (one span array per grid row) to an
    /// SGR stream with `\n` row separators (the delta reconciler still
    /// normalizes those to CRLF before feeding the view).
    public static func render(rows: [[Span]]) -> String {
        rows.map { renderRow($0) }.joined(separator: "\n")
    }

    /// Render an `am_spans_json` document, or nil when it does not
    /// decode (the pump then falls back to the plain-text snapshot).
    public static func render(json: String) -> String? {
        guard let data = json.data(using: .utf8),
              let rows = try? JSONDecoder().decode([[Span]].self, from: data)
        else { return nil }
        return render(rows: rows)
    }

    static func renderRow(_ spans: [Span]) -> String {
        spans.map { sgr($0) + $0.text }.joined()
    }

    /// Complete SGR sequence for one span: reset plus attributes, so the
    /// span renders the same standalone as inside the full snapshot.
    static func sgr(_ span: Span) -> String {
        var params = ["0"]
        if span.bold { params.append("1") }
        if span.italic { params.append("3") }
        if span.underline { params.append("4") }
        if let fg = rgb(span.fg) { params.append("38;2;\(fg)") }
        if let bg = rgb(span.bg) { params.append("48;2;\(bg)") }
        return "\u{1B}[" + params.joined(separator: ";") + "m"
    }

    static func rgb(_ triple: [Int]?) -> String? {
        guard let t = triple, t.count == 3 else { return nil }
        return "\(t[0]);\(t[1]);\(t[2])"
    }
}
