//! Markdown export of the selected run (issue #26, local-only).
//!
//! Key `e` in nav focus writes the run's visible text (live screen, or
//! the historic transcript when no PTY owns the run) plus every retained
//! link to a markdown file under the local data dir, then flashes the
//! path. No account, no sync — a plain file on this machine.

use std::path::PathBuf;

use crate::app::ChatSession;
use crate::transcript::Role;

use super::shell::ShellView;

/// One-line role label for historic transcript rows.
fn role_label(role: Role) -> &'static str {
    match role {
        Role::User => "you",
        Role::Assistant => "muse",
        Role::Tool => "tool",
        Role::System => "sys",
        Role::Unknown => "",
    }
}

/// Plain-text rows of a historic run's transcript, for markdown export
/// when no live PTY owns the run.
pub(crate) fn transcript_rows(session: &ChatSession) -> Vec<String> {
    if session.transcript.is_empty() {
        return vec!["(no transcript captured for this run)".to_string()];
    }
    session
        .transcript
        .iter()
        .map(|m| {
            let label = role_label(m.role);
            if label.is_empty() {
                m.text.clone()
            } else {
                format!("{label}: {}", m.text)
            }
        })
        .collect()
}

impl ShellView {
    /// Export the selected run and flash the path. Returns the file path
    /// on success (`None` when no run is selected or the write failed,
    /// with a sticky error in the latter case).
    pub(crate) fn export_selected_run(&mut self) -> Option<PathBuf> {
        let session = self.app.selected_session()?.clone();
        let body = match self.active_view() {
            Some(view) => view.screen.contents(),
            None => transcript_rows(&session).join("\n"),
        };
        match crate::persist::write_export(&session, &body, &crate::persist::exports_dir()) {
            Ok(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.app.set_status(format!("exported {name}"));
                Some(path)
            }
            Err(e) => {
                self.app.set_error(format!("export failed: {e}"));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::runs::{insert_test_pty, test_shell};

    #[test]
    fn export_writes_screen_text_plus_links_locally() {
        let mut view = test_shell();
        let id = insert_test_pty(&mut view, "printf", &["waiting for approval\\n"]);
        for _ in 0..100 {
            view.refresh();
            if let Some(v) = view.active_view() {
                if v.screen.contents().contains("approval") {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let s = view.app.sessions.iter_mut().find(|s| s.id == id).unwrap();
        s.pr_links
            .push("https://github.com/acme/app/pull/42".to_string());
        // Redirect the export into a temp dir by hand: the action uses the
        // default dir, so verify content through the shared writer here
        // and the action's flash through the default path.
        let dir = std::env::temp_dir().join(format!(
            "staap-export-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let session = view.app.selected_session().unwrap().clone();
        let body = view.active_view().unwrap().screen.contents();
        let path = crate::persist::write_export(&session, &body, &dir).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("waiting for approval"),
            "screen text survives"
        );
        assert!(
            text.contains("https://github.com/acme/app/pull/42"),
            "links survive"
        );
        std::fs::remove_dir_all(&dir).ok();
        // The action itself flashes the filename (default export dir).
        let flashed = view.export_selected_run().expect("export succeeds");
        assert!(view.app.status_text().unwrap().starts_with("exported "));
        std::fs::remove_file(&flashed).ok();
    }

    #[test]
    fn export_without_a_run_is_a_quiet_noop() {
        let mut view = test_shell();
        assert!(view.export_selected_run().is_none());
    }
}
