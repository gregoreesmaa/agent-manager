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

        // Shared-helper surface: key table, reconciler, preview, yolo,
        // age, glyphs, headers, clamp, filter/selection — one copy in
        // the core, same on every shell.
        guard Core.keyEncode(key: "enter", keyChar: nil, ctrl: false, alt: false) == [0x0D] else {
            return fail("am_key_encode enter != CR")
        }
        guard Core.keyEncode(key: "shift", keyChar: nil, ctrl: false, alt: false) == nil else {
            return fail("am_key_encode shift should Keep")
        }
        guard Core.feedDelta(old: "a", new: "a\nb\n") == "\r\nb\r\n" else {
            return fail("am_feed_delta append")
        }
        guard Core.feedDelta(old: "same", new: "same") == nil else {
            return fail("am_feed_delta identical != nil")
        }
        let spansDoc = #"[[{"text":"red","fg":[205,0,0],"bg":null,"bold":false,"italic":false,"underline":false}]]"#
        guard Core.ansiRender(json: spansDoc) == "\u{1B}[0;38;2;205;0;0mred" else {
            return fail("am_ansi_render red span")
        }
        guard Core.spawnPreview(cli: "muse", folder: "/tmp/api", yolo: 1) == "runs: muse in /tmp/api + yolo" else {
            return fail("am_spawn_preview")
        }
        guard Core.yoloValue(1) == 1, Core.yoloValue(2) == -1 else {
            return fail("am_yolo_value")
        }
        guard Core.ageString(now: 600, then: 0) == "10m ago" else {
            return fail("am_age_string")
        }
        guard Core.statusGlyph(0) == "●" else {
            return fail("am_status_glyph")
        }
        guard Core.sectionTitle(2) == "Working" else {
            return fail("am_section_title")
        }
        guard Core.maxRuns == 10 else {
            return fail("am_max_runs != 10")
        }
        guard core.liveCount == 0 else {
            return fail("fresh core has live runs")
        }

        print("SMOKE-OK sessions=\(n)")

        // Opt-in live converse proof against the real agent command:
        // registry spawn, pump for output, write (no newline: echoed or
        // buffered, never submitted), resize, close. Bounded by timeouts.
        if CommandLine.arguments.contains("--smoke-live") {
            guard liveConverse(core: core, fail: fail) else { return false }
        }
        return true
    }

    static func liveConverse(core: Core, fail: (String) -> Bool) -> Bool {
        let id: String
        do {
            id = try core.runSpawn(cli: nil, cwd: nil, yolo: 0, cols: 80, rows: 24)
        } catch {
            return fail("live spawn: \(error.localizedDescription)")
        }
        defer { core.runClose(id: id) }

        // Pump until the child produces visible output.
        var firstBytes = 0
        let start = Date()
        while Date().timeIntervalSince(start) < 15 {
            if core.runPump(id: id), let text = core.runScreenText(id: id) {
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
            try core.runWrite(id: id, bytes: Array("smoke-probe-62".utf8))
        } catch {
            return fail("live write: \(error.localizedDescription)")
        }

        // Resize keeps the seam alive; the screen stays readable.
        core.runResize(id: id, cols: 100, rows: 30)
        Thread.sleep(forTimeInterval: 0.3)
        _ = core.runPump(id: id)
        guard let after = core.runScreenText(id: id), !after.isEmpty else {
            return fail("live: unreadable screen after resize")
        }

        print("SMOKE-LIVE-OK firstBytes=\(firstBytes)")
        return true
    }
}
