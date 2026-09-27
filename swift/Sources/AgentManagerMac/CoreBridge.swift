import Foundation

// Swift declarations of the core staticlib C ABI. The canonical
// contract is `include/agent_manager.h` at the repo root; these
// signatures must match it exactly (name, arity, and C type mapping:
// `size_t` -> Int, `_Bool` -> Bool, `char *` -> CChar pointer).
@_silgen_name("am_core_new") private func am_core_new() -> OpaquePointer?
@_silgen_name("am_core_free") private func am_core_free(_ core: OpaquePointer?)
@_silgen_name("am_core_save") private func am_core_save(_ core: OpaquePointer?) -> Int32
@_silgen_name("am_session_count") private func am_session_count(_ core: OpaquePointer?) -> Int
@_silgen_name("am_session_json") private func am_session_json(
    _ core: OpaquePointer?, _ row: Int
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("am_spawn") private func am_spawn(
    _ core: OpaquePointer?,
    _ out: UnsafeMutablePointer<OpaquePointer?>,
    _ cwd: UnsafePointer<CChar>?,
    _ cols: UInt16,
    _ rows: UInt16
) -> Int32
@_silgen_name("am_pump") private func am_pump(_ pty: OpaquePointer?) -> Bool
@_silgen_name("am_write") private func am_write(
    _ pty: OpaquePointer?, _ data: UnsafePointer<UInt8>?, _ len: Int
) -> Int32
@_silgen_name("am_resize") private func am_resize(
    _ pty: OpaquePointer?, _ cols: UInt16, _ rows: UInt16
)
@_silgen_name("am_screen_text") private func am_screen_text(
    _ pty: OpaquePointer?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("am_screen_text_free") private func am_screen_text_free(
    _ s: UnsafeMutablePointer<CChar>?
)
@_silgen_name("am_status") private func am_status(
    _ core: OpaquePointer?, _ row: Int
) -> Int32
@_silgen_name("am_last_error") private func am_last_error() -> UnsafePointer<CChar>
@_silgen_name("am_pty_free") private func am_pty_free(_ pty: OpaquePointer?)

/// Copy an owned core string into Swift, then free the core side.
private func copyOwnedString(_ ptr: UnsafeMutablePointer<CChar>?) -> String? {
    guard let ptr else { return nil }
    defer { am_screen_text_free(ptr) }
    return String(validatingUTF8: ptr)
}

/// Errors from the fallible C ABI calls, carrying the core's
/// thread-local message for display.
enum CoreError: Error, LocalizedError {
    case spawn(message: String)
    case write(message: String)
    case save(message: String)

    var errorDescription: String? {
        switch self {
        case let .spawn(m): "Could not start session: \(m)"
        case let .write(m): "Could not send input: \(m)"
        case let .save(m): "Could not save: \(m)"
        }
    }
}

/// One roster row decoded from the core's `ChatSession` JSON
/// (`am_session_json`). Only the fields the shell renders are kept.
struct SessionRow: Identifiable, Decodable {
    var id: String
    var title: String
    var project: String
    var statusName: String
    var harness: String
    var lastActive: Int64
    var cwd: String?
    var prLinks: [String]?
    var relatedLinks: [String]?

    enum CodingKeys: String, CodingKey {
        case id
        case title
        case project
        case statusName = "status"
        case harness
        case lastActive = "last_active"
        case cwd
        case prLinks = "pr_links"
        case relatedLinks = "related_links"
    }

    var linkCount: Int { (prLinks?.count ?? 0) + (relatedLinks?.count ?? 0) }
}

/// Roster status codes, matching `am_status` (Attention=0, Idle=1,
/// Working=2; negatives are null handle / out of bounds).
enum RunStatus: Int {
    case attention = 0
    case idle = 1
    case working = 2
}

/// Owned wrapper around the core's opaque roster handle.
final class Core {
    private let handle: OpaquePointer

    init?() {
        guard let handle = am_core_new() else { return nil }
        self.handle = handle
    }

    deinit { am_core_free(handle) }

    static func lastError() -> String {
        String(cString: am_last_error())
    }

    var sessionCount: Int { am_session_count(handle) }

    func sessionRow(_ row: Int) -> SessionRow? {
        guard let text = copyOwnedString(am_session_json(handle, row)),
              let data = text.data(using: .utf8)
        else { return nil }
        return try? JSONDecoder().decode(SessionRow.self, from: data)
    }

    func status(_ row: Int) -> Int32 { am_status(handle, row) }

    func save() throws {
        if am_core_save(handle) != 0 {
            throw CoreError.save(message: Self.lastError())
        }
    }

    func spawn(cols: Int, rows: Int) throws -> Pty {
        var out: OpaquePointer?
        let rc = am_spawn(
            handle, &out, nil,
            UInt16(clamping: cols), UInt16(clamping: rows)
        )
        guard rc == 0, let handle = out else {
            throw CoreError.spawn(message: Self.lastError())
        }
        return Pty(handle: handle)
    }
}

/// Owned wrapper around one live session PTY.
final class Pty {
    fileprivate let handle: OpaquePointer

    init(handle: OpaquePointer) { self.handle = handle }

    deinit { am_pty_free(handle) }

    /// Feed queued output into the core emulator. Returns true when the
    /// screen may have changed (the shell's only repaint gate).
    func pump() -> Bool { am_pump(handle) }

    /// Forward key-encoded bytes to the child.
    func write(_ bytes: [UInt8]) throws {
        let rc = bytes.withUnsafeBufferPointer { buf in
            am_write(handle, buf.baseAddress, buf.count)
        }
        if rc != 0 {
            throw CoreError.write(message: Core.lastError())
        }
    }

    func resize(cols: Int, rows: Int) {
        am_resize(handle, UInt16(clamping: cols), UInt16(clamping: rows))
    }

    /// Owned plain-text snapshot of the emulated screen.
    func screenText() -> String? {
        copyOwnedString(am_screen_text(handle))
    }
}
