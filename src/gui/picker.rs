//! Two-dimensional new-session picker (folder × CLI + yolo) for the gpui shell.
//!
//! Split-button model: `n` / the `+ New` main click repeats the last launch
//! instantly (expert fast path, 1 keystroke, unchanged); `N` / the `▾` caret
//! / the empty-pane picker button opens this capture, where the folder axis
//! and the CLI axis are picked independently before Enter confirms.
//!
//! Thin by contract: the keyboard state machine lives in the framework-free
//! `crate::launch::LaunchPicker` (tested headlessly on every OS gate); this
//! module only opens/accepts/cancels it and renders the preview/status copy
//! the shell shows. Rendering (a follow-up over `runs_panel.rs`) reads the
//! same state natively.

use crate::launch::LaunchSelection;

use super::shell::ShellView;

impl ShellView {
    /// Picker capture state (2D launch): `Some` while the picker owns the
    /// keyboard. `None` is every other mode (filter/folder captures,
    /// terminal, nav). Read by nav dispatch tests and (on macOS) the
    /// status-bar render.
    pub(crate) fn picker_open(&self) -> bool {
        self.launch_picker.is_some()
    }

    /// Open the 2D picker (`N` in nav focus, the `▾` caret, the empty-pane
    /// picker button): autodetect fresh on every open so the CLI list is
    /// never stale, preselecting the configured default / last-used CLI
    /// and the default folder. No-op when already open.
    pub(crate) fn open_launch_picker(&mut self) {
        if self.launch_picker.is_some() {
            return;
        }
        let clis = crate::launch::detect_available_clis();
        if clis.is_empty() {
            // Unreachable in practice (the catalog always lists all four):
            // kept fail-visible instead of opening an empty picker.
            self.app.set_error(
                "no supported agent CLI found on PATH (muse, claude, opencode, codex) · install one, then N: retry",
            );
            return;
        }
        let default_cli = self.app.config_default_cli().map(str::to_string);
        let last_cli = self.app.config_last_cli().map(str::to_string);
        let effective = crate::launch::resolve_effective_cli(
            None,
            last_cli.as_deref(),
            default_cli.as_deref(),
            &clis,
        );
        let default_cwd = self.app.config_default_cwd().map(str::to_string);
        let recents = self.app.config_recents().to_vec();
        self.launch_picker = Some(crate::launch::LaunchPicker::new(
            clis,
            Some(&effective),
            default_cwd.as_deref(),
            recents,
        ));
    }

    /// Close the picker with zero side effects (Esc, Cmd+1/2 pane jumps):
    /// no session starts, memory untouched.
    pub(crate) fn cancel_launch_picker(&mut self) {
        self.launch_picker = None;
    }

    /// Confirm the picker (Enter): validate the folder (blank inherits),
    /// start the session through the shared `App::start_launch` funnel,
    /// flash the `runs: <preview>` confirmation. A missing folder refuses
    /// with a flash and stays open for a fix. Returns true on success.
    pub(crate) fn accept_launch_picker(&mut self) -> bool {
        let Some(picker) = self.launch_picker.clone() else {
            return false;
        };
        let expanded = crate::app::expand_cwd_input(&picker.folder_input);
        if let Some(dir) = &expanded {
            if !std::path::Path::new(dir).is_dir() {
                self.app.set_status(format!("no such folder: {dir}"));
                return false;
            }
        }
        let yolo_default = self
            .app
            .config_yolo_default(picker.selected_cli().id.as_str());
        let selection = LaunchSelection::new(
            picker.selected_cli().id.clone(),
            expanded,
            picker.yolo.resolve(yolo_default),
        );
        self.launch_picker = None;
        self.link_cursor = None;
        if !self.request_new_launch(selection.clone()) {
            return false;
        }
        #[cfg(not(test))]
        let _ = self.app.save_config();
        let yolo_tag = if selection.yolo { " · yolo" } else { "" };
        self.app
            .set_status(format!("runs: {}{yolo_tag}", selection.preview()));
        true
    }

    /// Picker key dispatch: Enter confirms, Esc cancels, everything else
    /// forwards to the framework-free [`crate::launch::LaunchPicker::key`]
    /// state machine. Returns true when the picker owned the key.
    pub(crate) fn picker_key(
        &mut self,
        key: &str,
        key_char: Option<&str>,
        ctrl: bool,
        platform: bool,
    ) -> bool {
        if self.launch_picker.is_none() {
            return false;
        }
        match key {
            "enter" => {
                self.accept_launch_picker();
            }
            "escape" => {
                self.cancel_launch_picker();
            }
            _ => {
                if let Some(picker) = self.launch_picker.as_mut() {
                    picker.key(key, key_char, ctrl, platform);
                }
            }
        }
        true
    }

    /// Preview line for the picker footer (`runs: muse --yolo in ~/api`):
    /// the exact spawn, verifiable before it runs.
    pub(crate) fn picker_preview(&self) -> String {
        match &self.launch_picker {
            Some(picker) => {
                let yolo_default = self
                    .app
                    .config_yolo_default(picker.selected_cli().id.as_str());
                format!("runs: {}", picker.preview(yolo_default))
            }
            None => String::new(),
        }
    }

    /// Yolo label for the picker row (text, never color-only).
    pub(crate) fn picker_yolo_label(&self) -> &'static str {
        match &self.launch_picker {
            Some(picker) => picker.yolo.label(),
            None => crate::launch::YoloChoice::UseDefault.label(),
        }
    }
}
