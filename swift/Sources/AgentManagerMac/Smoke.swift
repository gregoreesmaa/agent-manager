import Foundation

/// Headless runtime check of the linked core staticlib. Exercises the
/// read-only C ABI surface end to end (no spawn, no config writes, no
/// UI) and prints one line for harnesses to grep:
///
///     SMOKE-OK sessions=<n>
///
/// Exit status is 0 on success, 1 on the first failure.
enum Smoke {
    static func run() -> Bool {
        func fail(_ what: String) -> Bool {
            FileHandle.standardError.write(Data("SMOKE-FAIL \(what)\n".utf8))
            return false
        }

        guard let core = Core() else { return fail("am_core_new NULL") }
        let n = core.sessionCount

        // Every row the count reports must decode with a valid status.
        for i in 0 ..< n {
            guard core.sessionRow(i) != nil else {
                return fail("row \(i) does not decode")
            }
            let st = core.status(i)
            if st < 0 || st > 2 { return fail("row \(i) status \(st)") }
        }

        // Out-of-bounds row: -2 status, nil JSON, message recorded.
        let oob = n + 1_000_000
        if core.status(oob) != -2 { return fail("am_status OOB != -2") }
        if core.sessionRow(oob) != nil { return fail("am_session_json OOB non-nil") }
        if Core.lastError().isEmpty { return fail("am_last_error empty after failure") }

        print("SMOKE-OK sessions=\(n)")

        // Opt-in live converse proof against the real agent command:
        // spawn, pump for output, write (no newline: echoed or buffered,
        // never submitted), resize, free. Bounded by timeouts.
        if CommandLine.arguments.contains("--smoke-live") {
            guard liveConverse(core: core, fail: fail) else { return false }
        }
        return true
    }

    static func liveConverse(core: Core, fail: (String) -> Bool) -> Bool {
        let pty: Pty
        do {
            pty = try core.spawn(cols: 80, rows: 24)
        } catch {
            return fail("live spawn: \(error.localizedDescription)")
        }
        defer { _ = pty } // am_pty_free on scope exit (reaps the child)

        // Pump until the child produces visible output.
        var firstBytes = 0
        let start = Date()
        while Date().timeIntervalSince(start) < 15 {
            if pty.pump(), let text = pty.screenText() {
                let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty {
                    firstBytes = text.utf8.count
                    break
                }
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
        if firstBytes == 0 { return fail("live: no output within 15s") }

        // Input reaches the PTY: the write must succeed at the fd level.
        do {
            try pty.write(Array("smoke-probe-62".utf8))
        } catch {
            return fail("live write: \(error.localizedDescription)")
        }

        // Resize keeps the seam alive; the screen stays readable.
        pty.resize(cols: 100, rows: 30)
        Thread.sleep(forTimeInterval: 0.3)
        _ = pty.pump()
        guard let after = pty.screenText(), !after.isEmpty else {
            return fail("live: unreadable screen after resize")
        }

        print("SMOKE-LIVE-OK firstBytes=\(firstBytes)")
        return true
    }
}
