//! Agent models, conversation status, and sort order.

use serde::{Deserialize, Serialize};

/// Conversation attention state, ordered by urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Status {
    /// Needs user attention (e.g. approval prompt, error, recent question).
    Attention,
    /// Waiting / no recent activity.
    Idle,
    /// Actively producing output.
    Working,
}

/// One chat/agent conversation surfaced by a [`crate::providers::Provider`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub project: String,
    pub status: Status,
    /// Unix seconds of last observed activity (for display only).
    pub last_active: i64,
    /// GitHub PR URLs extracted from the conversation transcript.
    pub pr_links: Vec<String>,
}

impl ChatSession {
    pub fn status_label(&self) -> &'static str {
        match self.status {
            Status::Attention => "attention",
            Status::Working => "working",
            Status::Idle => "idle",
        }
    }
}

/// Top-down sort: attention first, then idle, then working; stable by
/// `last_active` descending within a status bucket.
pub fn sort_sessions(sessions: &mut [ChatSession]) {
    sessions.sort_by(|a, b| {
        a.status
            .cmp(&b.status)
            .then_with(|| b.last_active.cmp(&a.last_active))
    });
}

/// Application state: the session list plus list selection.
pub struct App {
    pub sessions: Vec<ChatSession>,
    pub selected: usize,
}

impl App {
    pub fn new(mut sessions: Vec<ChatSession>) -> Self {
        sort_sessions(&mut sessions);
        Self {
            sessions,
            selected: 0,
        }
    }

    pub fn select_next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.sessions.len();
    }

    pub fn select_prev(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .checked_sub(1)
            .unwrap_or(self.sessions.len() - 1);
    }

    pub fn selected_session(&self) -> Option<&ChatSession> {
        self.sessions.get(self.selected)
    }

    /// Distinct project names in first-seen order.
    pub fn projects(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for s in &self.sessions {
            if !out.contains(&s.project.as_str()) {
                out.push(s.project.as_str());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sess(id: &str, status: Status, last_active: i64) -> ChatSession {
        ChatSession {
            id: id.into(),
            title: id.into(),
            project: "proj".into(),
            status,
            last_active,
            pr_links: vec![],
        }
    }

    #[test]
    fn attention_sorts_first_then_idle_then_working() {
        let mut v = vec![
            sess("work", Status::Working, 99),
            sess("idle", Status::Idle, 1),
            sess("attn", Status::Attention, 1),
        ];
        sort_sessions(&mut v);
        assert_eq!(v[0].id, "attn");
        assert_eq!(v[1].id, "idle");
        assert_eq!(v[2].id, "work");
    }

    #[test]
    fn recency_breaks_status_ties() {
        let mut v = vec![
            sess("old", Status::Working, 1),
            sess("new", Status::Working, 5),
        ];
        sort_sessions(&mut v);
        assert_eq!(v[0].id, "new");
    }

    #[test]
    fn selection_wraps() {
        let mut app = App::new(vec![sess("a", Status::Idle, 1), sess("b", Status::Idle, 2)]);
        app.select_prev();
        assert_eq!(app.selected, 1);
        app.select_next();
        assert_eq!(app.selected, 0);
    }
}
