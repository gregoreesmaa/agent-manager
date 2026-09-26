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
}
