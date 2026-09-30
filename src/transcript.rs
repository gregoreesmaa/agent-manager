//! Chat transcript model + `session.jsonl` tail parsing.
//!
//! The on-disk session log is JSONL with per-record `payload_type` /
//! `payload` envelopes (see `providers::muse_cli`). This module extracts a
//! small ordered chat transcript from a capped tail of that log:
//!
//! - `runtime.user_intent.accepted` (and `run` / `started` prompts) → user
//! - `assistant_message_committed` / `reasoning_summary_committed` → assistant
//! - `assistant_tool_calls_committed` → assistant (tool-name summary)
//!
//! Caps: the provider caps the tail at [`MAX_TAIL_BYTES`]; parsing keeps at
//! most [`MAX_MESSAGES`] messages (the most recent) and [`MAX_MESSAGE_CHARS`]
//! chars per message. Anything dropped sets `truncated` on the result.

use serde::{Deserialize, Serialize};

/// Who produced a transcript message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
    Tool,
    System,
    Unknown,
}

/// One chat message extracted from the session log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptMessage {
    pub role: Role,
    pub text: String,
}

/// Ordered messages plus whether older content was dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedTranscript {
    pub messages: Vec<TranscriptMessage>,
    pub truncated: bool,
}

/// Max tail bytes the provider reads from `session.jsonl`.
pub const MAX_TAIL_BYTES: u64 = 64 * 1024;
/// Max messages kept per session (most recent win).
pub const MAX_MESSAGES: usize = 200;
/// Max chars kept per message.
pub const MAX_MESSAGE_CHARS: usize = 4000;
/// Max chars for derived single-line titles.
pub const MAX_TITLE_CHARS: usize = 60;

/// Parse a (possibly capped) `session.jsonl` tail into chat messages.
///
/// Lines that are not JSON are kept as [`Role::Unknown`] text so plain-text
/// transcripts still surface; unrecognized JSON envelopes are skipped.
pub fn parse_transcript(tail: &str) -> ParsedTranscript {
    let mut messages = Vec::new();
    for line in tail.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => extract_value(&value, &mut messages),
            Err(_) => push_capped(&mut messages, Role::Unknown, line.to_string()),
        }
    }
    let mut truncated = false;
    if messages.len() > MAX_MESSAGES {
        messages.drain(..messages.len() - MAX_MESSAGES);
        truncated = true;
    }
    ParsedTranscript {
        messages,
        truncated,
    }
}

/// Derive a single-line summary title: first user message, else first
/// non-empty message, else `fallback` (session auto-name or id).
pub fn derive_title(messages: &[TranscriptMessage], fallback: &str) -> String {
    let first = messages
        .iter()
        .find(|m| m.role == Role::User && !m.text.trim().is_empty())
        .or_else(|| messages.iter().find(|m| !m.text.trim().is_empty()));
    match first {
        Some(m) => {
            let one = single_line(&m.text);
            if one.is_empty() {
                single_line_max(fallback, MAX_TITLE_CHARS)
            } else {
                truncate_chars(&one, MAX_TITLE_CHARS)
            }
        }
        None => single_line_max(fallback, MAX_TITLE_CHARS),
    }
}

/// Collapse all whitespace (including newlines) to single spaces.
pub fn single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Best-effort project name: basename of the `workspace_root` in the first
/// `metadata` record of the tail.
pub fn extract_project(tail: &str) -> Option<String> {
    for line in tail.lines() {
        let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        let root = value
            .get("payload")?
            .get("record")?
            .get("workspace_root")?
            .as_str()?;
        if root.trim().is_empty() {
            continue;
        }
        let name = root
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(root);
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

/// opencode session tail: export-shaped JSON (`opencode export <id>`)
/// with `messages: [{info: {role}, parts: [{type, text}]}]` — or, on older
/// rows, a bare `messages` array of `{role, content}` objects. `text`
/// parts with non-empty text become chat messages; `tool` parts surface
/// as `🔧 <tool>` summaries; `step-start`/`step-finish` markers and other
/// scaffolding are skipped. Non-JSON lines are kept as [`Role::Unknown`]
/// (same contract as [`parse_transcript`).
pub fn parse_opencode_tail(tail: &str) -> ParsedTranscript {
    // Fast path: whole-tail export document.
    if let Ok(doc) = serde_json::from_str::<serde_json::Value>(
        tail.trim_start_matches(|c: char| c.is_whitespace()),
    ) {
        if doc.get("messages").and_then(|v| v.as_array()).is_some() {
            let mut messages = Vec::new();
            extract_opencode_doc(&doc, &mut messages);
            return cap_messages(messages);
        }
    }
    // Fallback: line-delimited message objects / plain text.
    let mut messages = Vec::new();
    for line in tail.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => extract_opencode_value(&value, &mut messages),
            Err(_) => push_capped(&mut messages, Role::Unknown, line.to_string()),
        }
    }
    cap_messages(messages)
}

fn cap_messages(mut messages: Vec<TranscriptMessage>) -> ParsedTranscript {
    let mut truncated = false;
    if messages.len() > MAX_MESSAGES {
        messages.drain(..messages.len() - MAX_MESSAGES);
        truncated = true;
    }
    ParsedTranscript {
        messages,
        truncated,
    }
}

/// Claude Code transcript tail: one JSON object per line with `type`
/// (`user`/`assistant`) and `message.content` text blocks. Non-JSON lines
/// are kept as [`Role::Unknown`] (same contract as [`parse_transcript`).
/// Tool-use/result entries surface as `🔧 <name>` summaries; anything else
/// (file-history, queue ops, summaries) is skipped.
pub fn parse_claude_tail(tail: &str) -> ParsedTranscript {
    let mut messages = Vec::new();
    for line in tail.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => extract_claude_value(&value, &mut messages),
            Err(_) => push_capped(&mut messages, Role::Unknown, line.to_string()),
        }
    }
    cap_messages(messages)
}

fn opencode_role(role: &str) -> Option<Role> {
    match role {
        "user" => Some(Role::User),
        "assistant" => Some(Role::Assistant),
        _ => None,
    }
}

fn opencode_part_text(role: Role, part: &serde_json::Value, out: &mut Vec<TranscriptMessage>) {
    match part.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "text" => {
            let text = part
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if !text.is_empty() {
                push_capped(out, role, text);
            }
        }
        "tool" => {
            let name = part
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .trim();
            if !name.is_empty() {
                push_capped(out, Role::Assistant, format!("🔧 {name}"));
            }
        }
        _ => {}
    }
}

fn extract_opencode_doc(doc: &serde_json::Value, out: &mut Vec<TranscriptMessage>) {
    let Some(messages) = doc.get("messages").and_then(|v| v.as_array()) else {
        return;
    };
    for m in messages {
        let role = m
            .get("info")
            .and_then(|i| i.get("role"))
            .or_else(|| m.get("role"))
            .and_then(|v| v.as_str())
            .and_then(opencode_role);
        let Some(role) = role else { continue };
        // Export shape: `parts: [{type, text}]`.
        if let Some(parts) = m.get("parts").and_then(|v| v.as_array()) {
            for part in parts {
                opencode_part_text(role, part, out);
            }
            continue;
        }
        // Compact shape: string or block-array `content`.
        match m.get("content") {
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
                push_capped(out, role, s.trim().to_string());
            }
            Some(serde_json::Value::Array(blocks)) => {
                for block in blocks {
                    if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                        let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                        push_capped(out, Role::Assistant, format!("🔧 {name}"));
                    } else if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        if !text.trim().is_empty() {
                            push_capped(out, role, text.trim().to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_opencode_value(value: &serde_json::Value, out: &mut Vec<TranscriptMessage>) {
    // Single message object (export shape or compact shape).
    if value.get("parts").is_some() || value.get("info").is_some() {
        let mut tmp = Vec::new();
        extract_opencode_doc(&serde_json::json!({"messages": [value]}), &mut tmp);
        out.extend(tmp);
        return;
    }
    if let Some(role) = value
        .get("role")
        .and_then(|v| v.as_str())
        .and_then(opencode_role)
    {
        match value.get("content") {
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
                push_capped(out, role, s.trim().to_string());
            }
            _ => {}
        }
    }
}

/// Best-effort resume handle for a Claude Code tail: the first `sessionId`
/// field found (every record carries it). Falls back to the log filename
/// at the provider layer.
pub fn extract_claude_session_id(tail: &str) -> Option<String> {
    for line in tail.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(id) = value.get("sessionId").and_then(|v| v.as_str()) {
            if !id.trim().is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Best-effort project name for a Claude Code tail: basename of the first
/// `cwd` field found (both `/` and `\` separators split, so Windows paths
/// work too).
pub fn extract_claude_project(tail: &str) -> Option<String> {
    for line in tail.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cwd) = value.get("cwd").and_then(|v| v.as_str()) else {
            continue;
        };
        if cwd.trim().is_empty() {
            continue;
        }
        let name = cwd
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(cwd);
        if !name.trim().is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

fn claude_text_blocks(content: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let blocks: Vec<&serde_json::Value> = match content {
        serde_json::Value::Array(items) => items.iter().collect(),
        obj => vec![obj],
    };
    for block in blocks {
        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
            if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                if !name.trim().is_empty() {
                    out.push(format!("🔧 {name}"));
                }
            }
            continue;
        }
        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
            if block
                .get("type")
                .and_then(|v| v.as_str())
                .is_none_or(|t| t == "text")
            {
                out.push(text.to_string());
            }
        }
    }
    out
}

fn extract_claude_value(value: &serde_json::Value, out: &mut Vec<TranscriptMessage>) {
    let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    // `user` / `assistant` carry `message: {role, content}`; content is
    // text blocks (kept, joined) or a bare string (newer compact rows).
    if kind == "user" || kind == "assistant" {
        let role = if kind == "user" {
            Role::User
        } else {
            Role::Assistant
        };
        let message = value.get("message");
        let content = message.and_then(|m| m.get("content")).or_else(|| {
            // Newer compact rows may inline content beside `type`.
            if value.get("content").is_some() {
                value.get("content")
            } else {
                None
            }
        });
        let text = match content {
            Some(serde_json::Value::String(s)) => s.trim().to_string(),
            Some(content) => claude_text_blocks(content).join("\n").trim().to_string(),
            None => String::new(),
        };
        if !text.is_empty() {
            push_capped(out, role, text);
        }
        return;
    }
    // Tool results and queue artifacts land after the assistant turn;
    // attribute emitted stdout to the assistant so it stays visible.
    if kind == "tool_result" {
        let text = value
            .get("toolUseResult")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("content").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim()
            .to_string();
        if !text.is_empty() {
            push_capped(out, Role::Assistant, text);
        }
    }
}

/// Best-effort project name for an opencode export tail: basename of the
/// top-level `info.directory` (both `/` and `\` split, so Windows paths
/// work too).
pub fn extract_opencode_project(tail: &str) -> Option<String> {
    let doc: serde_json::Value = serde_json::from_str(tail.trim_start()).ok()?;
    let dir = doc
        .get("info")
        .and_then(|i| i.get("directory"))
        .and_then(|v| v.as_str())?;
    if dir.trim().is_empty() {
        return None;
    }
    let name = dir
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(dir);
    if name.trim().is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Best-effort session title for an opencode export tail: the top-level
/// `info.title` opencode itself assigns.
pub fn extract_opencode_title(tail: &str) -> Option<String> {
    let doc: serde_json::Value = serde_json::from_str(tail.trim_start()).ok()?;
    doc.get("info")
        .and_then(|i| i.get("title"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
}

/// Best-effort session auto-name (`session.name.changed` record), for use as
/// a title fallback when the tail holds no messages.
pub fn extract_session_name(tail: &str) -> Option<String> {
    let mut name = None;
    for line in tail.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if value.get("payload_type").and_then(|v| v.as_str()) != Some("session.name.changed") {
            continue;
        }
        if let Some(new) = value
            .get("payload")
            .and_then(|p| p.get("new_name"))
            .and_then(|n| n.as_str())
        {
            if !new.trim().is_empty() {
                name = Some(new.to_string());
            }
        }
    }
    name
}

fn extract_value(value: &serde_json::Value, out: &mut Vec<TranscriptMessage>) {
    // Retained-frame wrapper: recurse into embedded record JSON strings.
    if let Some(children) = value.get("children").and_then(|v| v.as_array()) {
        for child in children {
            if let Some(inner) = child.get("record_json").and_then(|v| v.as_str()) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(inner) {
                    extract_value(&parsed, out);
                }
            }
        }
        return;
    }
    let payload_type = value
        .get("payload_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Some(payload) = value.get("payload") else {
        return;
    };
    if payload_type == "runtime.user_intent.accepted" {
        let mut texts = Vec::new();
        if let Some(model_messages) = payload.get("model_messages").and_then(|v| v.as_array()) {
            for message in model_messages {
                if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                    for block in content {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            texts.push(text);
                        }
                    }
                }
            }
        }
        if texts.is_empty() {
            if let Some(blocks) = payload.get("refill_blocks").and_then(|v| v.as_array()) {
                for block in blocks {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        texts.push(text);
                    }
                }
            }
        }
        let joined = texts.join("\n");
        if !joined.trim().is_empty() {
            push_capped(out, Role::User, joined.trim().to_string());
        }
        return;
    }
    if payload.get("kind").and_then(|v| v.as_str()) != Some("run") {
        return;
    }
    let Some(event) = payload.get("event") else {
        return;
    };
    match event.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
        "started" => {
            if let Some(prompt) = event.get("prompt").and_then(|v| v.as_str()) {
                if !prompt.trim().is_empty() {
                    push_capped(out, Role::User, prompt.trim().to_string());
                }
            }
        }
        "assistant_message_committed" | "reasoning_summary_committed" => {
            if let Some(text) = event.get("text").and_then(|v| v.as_str()) {
                if !text.trim().is_empty() {
                    push_capped(out, Role::Assistant, text.trim().to_string());
                }
            }
        }
        "assistant_tool_calls_committed" => {
            if let Some(calls) = event.get("tool_calls").and_then(|v| v.as_array()) {
                let names: Vec<String> = calls
                    .iter()
                    .map(|c| {
                        c.get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool")
                            .to_string()
                    })
                    .collect();
                if !names.is_empty() {
                    push_capped(out, Role::Assistant, format!("🔧 {}", names.join(", ")));
                }
            }
        }
        _ => {}
    }
}

fn push_capped(out: &mut Vec<TranscriptMessage>, role: Role, text: String) {
    let text = truncate_chars(&text, MAX_MESSAGE_CHARS);
    // `run started` prompts duplicate the accepted intent text; skip adjacent
    // identical messages so the transcript does not stutter.
    if out.last().is_some_and(|m| m.role == role && m.text == text) {
        return;
    }
    out.push(TranscriptMessage { role, text });
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

fn single_line_max(s: &str, max: usize) -> String {
    truncate_chars(&single_line(s), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent_line(text: &str) -> String {
        serde_json::json!({
            "payload_type": "runtime.user_intent.accepted",
            "payload": {
                "model_messages": [{"content": [{"kind": "text", "text": text}]}]
            }
        })
        .to_string()
    }

    fn assistant_line(text: &str) -> String {
        serde_json::json!({
            "payload_type": "runtime.session",
            "payload": {
                "kind": "run",
                "run_id": "r1",
                "event": {
                    "kind": "assistant_message_committed",
                    "message_id": "m",
                    "response_id": "x",
                    "text": text
                }
            }
        })
        .to_string()
    }

    #[test]
    fn parses_user_and_assistant_roles_in_order() {
        let tail = format!("{}\n{}\n", intent_line("Fix login"), assistant_line("Done"));
        let parsed = parse_transcript(&tail);
        assert!(!parsed.truncated);
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, Role::User);
        assert_eq!(parsed.messages[0].text, "Fix login");
        assert_eq!(parsed.messages[1].role, Role::Assistant);
        assert_eq!(parsed.messages[1].text, "Done");
    }

    #[test]
    fn dedupes_run_started_prompt_against_accepted_intent() {
        let started = serde_json::json!({
            "payload_type": "runtime.session",
            "payload": {
                "kind": "run",
                "run_id": "r1",
                "event": {"kind": "started", "prompt": "Fix login"}
            }
        })
        .to_string();
        let tail = format!("{}\n{started}\n", intent_line("Fix login"));
        let parsed = parse_transcript(&tail);
        assert_eq!(parsed.messages.len(), 1);
    }

    #[test]
    fn caps_message_count_and_marks_truncated() {
        let mut tail = String::new();
        for i in 0..MAX_MESSAGES + 50 {
            tail.push_str(&intent_line(&format!("msg {i}")));
            tail.push('\n');
        }
        let parsed = parse_transcript(&tail);
        assert!(parsed.truncated);
        assert_eq!(parsed.messages.len(), MAX_MESSAGES);
        // Most recent win: the first 50 were dropped.
        assert_eq!(parsed.messages[0].text, "msg 50");
        assert_eq!(
            parsed.messages[MAX_MESSAGES - 1].text,
            format!("msg {}", MAX_MESSAGES + 49)
        );
    }

    #[test]
    fn title_is_single_line_and_capped() {
        let messages = vec![TranscriptMessage {
            role: Role::User,
            text: "Fix   login\nsecond   line\twith tabs".into(),
        }];
        let title = derive_title(&messages, "fallback-id");
        assert!(!title.contains('\n'));
        assert!(!title.contains('\t'));
        assert!(!title.contains("  "));
        assert_eq!(title, "Fix login second line with tabs");
    }

    #[test]
    fn long_title_truncates_with_marker() {
        let messages = vec![TranscriptMessage {
            role: Role::User,
            text: "x".repeat(200),
        }];
        let title = derive_title(&messages, "fallback-id");
        assert!(!title.contains('\n'));
        assert!(title.chars().count() <= MAX_TITLE_CHARS + 1);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn empty_transcript_uses_fallback_title() {
        assert_eq!(derive_title(&[], "sess-1"), "sess-1");
    }

    #[test]
    fn plain_text_lines_become_unknown_messages() {
        let parsed = parse_transcript("Fix login\nsee https://github.com/acme/app/pull/9\n");
        assert_eq!(parsed.messages.len(), 2);
        assert!(parsed.messages.iter().all(|m| m.role == Role::Unknown));
        assert_eq!(derive_title(&parsed.messages, "id"), "Fix login");
    }

    #[test]
    fn extracts_project_basename_from_metadata() {
        let tail = serde_json::json!({
            "payload_type": "runtime.session.metadata",
            "payload": {"kind": "metadata", "record": {"workspace_root": "/tmp/xyz/myproj"}}
        })
        .to_string();
        assert_eq!(extract_project(&tail).as_deref(), Some("myproj"));
        assert_eq!(extract_project("not json\n"), None);
    }

    #[test]
    fn parses_opencode_export_doc_in_order() {
        let tail = serde_json::json!({
            "info": {"id": "ses-1", "directory": "/tmp/work/shop",
                "title": "Claude, Codex, Antigravity CLI worktree PRs"},
            "messages": [
                {"info": {"role": "user"},
                 "parts": [{"type": "text", "text": "Fix login"}]},
                {"info": {"role": "assistant"},
                 "parts": [
                    {"type": "step-start"},
                    {"type": "text", "text": "Done"},
                    {"type": "tool", "tool": "read"},
                    {"type": "step-finish"},
                ]},
            ]
        })
        .to_string();
        let parsed = parse_opencode_tail(&tail);
        assert!(!parsed.truncated);
        assert_eq!(parsed.messages.len(), 3);
        assert_eq!(parsed.messages[0].role, Role::User);
        assert_eq!(parsed.messages[0].text, "Fix login");
        assert_eq!(parsed.messages[1].role, Role::Assistant);
        assert_eq!(parsed.messages[1].text, "Done");
        assert_eq!(parsed.messages[2].text, "🔧 read");
        // Titles prefer opencode's own; project is the directory basename.
        assert_eq!(
            extract_opencode_title(&tail).as_deref(),
            Some("Claude, Codex, Antigravity CLI worktree PRs")
        );
        assert_eq!(extract_opencode_project(&tail).as_deref(), Some("shop"));
        assert_eq!(extract_opencode_project("not json\n"), None);
        assert_eq!(extract_opencode_title("not json\n"), None);
    }

    #[test]
    fn opencode_windows_directory_projects_split_on_backslash() {
        let tail =
            r#"{"info": {"directory": "C:\\Users\\grego\\projects\\agent-manager", "title": "t"}}"#;
        assert_eq!(
            extract_opencode_project(tail).as_deref(),
            Some("agent-manager")
        );
    }

    #[test]
    fn opencode_plain_text_lines_become_unknown_messages() {
        let parsed = parse_opencode_tail("Fix login\n");
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].role, Role::Unknown);
    }

    #[test]
    fn parses_claude_user_and_assistant_blocks_in_order() {
        let tail = [
            serde_json::json!({
                "type": "user",
                "sessionId": "s-1",
                "cwd": "/tmp/work/shop",
                "message": {"role": "user",
                    "content": [{"type": "text", "text": "Fix login"}]}
            }),
            serde_json::json!({
                "type": "assistant",
                "sessionId": "s-1",
                "cwd": "/tmp/work/shop",
                "message": {"role": "assistant",
                    "content": [{"type": "text", "text": "Done"}]}
            }),
        ]
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let parsed = parse_claude_tail(&tail);
        assert!(!parsed.truncated);
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, Role::User);
        assert_eq!(parsed.messages[0].text, "Fix login");
        assert_eq!(parsed.messages[1].role, Role::Assistant);
        assert_eq!(parsed.messages[1].text, "Done");
        assert_eq!(extract_claude_session_id(&tail).as_deref(), Some("s-1"));
        assert_eq!(extract_claude_project(&tail).as_deref(), Some("shop"));
        assert_eq!(extract_claude_project("not json\n"), None);
        assert_eq!(extract_claude_session_id("not json\n"), None);
    }

    #[test]
    fn claude_tool_use_blocks_surface_as_tool_summaries() {
        let tail = serde_json::json!({
            "type": "assistant",
            "sessionId": "s-2",
            "cwd": "C:\\work\\myproj",
            "message": {"role": "assistant",
                "content": [{"type": "tool_use", "name": "Edit"}]}
        })
        .to_string();
        let parsed = parse_claude_tail(&tail);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].role, Role::Assistant);
        assert_eq!(parsed.messages[0].text, "🔧 Edit");
        // Windows path projects split on backslashes too.
        assert_eq!(extract_claude_project(&tail).as_deref(), Some("myproj"));
    }

    #[test]
    fn claude_bare_string_content_and_plain_text_lines_parse() {
        let tail = serde_json::json!({
            "type": "user",
            "sessionId": "s-3",
            "message": {"content": "Fix login"}
        })
        .to_string();
        let parsed = parse_claude_tail(&format!("{tail}\nplain line\n"));
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, Role::User);
        assert_eq!(parsed.messages[0].text, "Fix login");
        assert_eq!(parsed.messages[1].role, Role::Unknown);
    }
}
