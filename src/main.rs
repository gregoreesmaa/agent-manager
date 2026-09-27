//! agent-manager: native GUI harness for live `muse` runs (gpui, no webview).
//!
//! Left panel: live runs started in-app (`n` or the New button); starts
//! empty, shows animal placeholder titles until the first submitted prompt
//! renames a run, groups by Needs input / Idle / Active with counts, and
//! lists every GitHub PR link ever seen in each run (accumulated, not just
//! the visible screen). Central pane: the embedded interactive `muse`
//! terminal, rendered from the vt100 emulator grid; drag to highlight text
//! (copy-on-select), Cmd+C copies, Cmd/Ctrl+V pastes.
//! Keys (nav focus): j/k move, n new, y copy selection-or-screen, p paste,
//! Tab/i type, ? help, q quit. Typing focus: keys go to `muse`; Tab/Esc back to
//! the list; Cmd+1/Cmd+2 jump to either pane from anywhere.

mod app;
mod config;
mod embedded;
mod gui;
mod parsers;
mod persist;
mod providers;
mod scrollback;
mod transcript;

use gpui::{AppContext, Application, Entity};
use gpui_component::{Root, Theme};

use gui::shell::ShellView;
use providers::{MuseCliProvider, Provider};

fn main() {
    // Historic attach: provider-discovered sessions seed the list before
    // any PTY exists (unreachable store degrades to an empty list, never
    // a startup failure). `r` on a seeded entry re-attaches it.
    let discovered = MuseCliProvider::new(
        MuseCliProvider::default_store_root(),
        Box::new(parsers::registry::RegistryParser::default()),
    )
    .discover_sessions();
    // Issues #33/#34: user config (per-agent flags, theme choice); a
    // missing file means plain `muse` + follow-system theme.
    let config = config::Config::load();
    let startup_theme = config.theme;
    // Persisted links/transcripts merge under discovery (issue #26):
    // a missing or corrupt file degrades to discovery alone.
    let seeded = persist::merge_sessions(discovered, persist::load_sessions());
    Application::new().run(move |cx| {
        // Native chrome components (sidebar, buttons): init once. The
        // theme applies per window below so `system` can follow the OS.
        gpui_component::init(cx);
        let view: Entity<ShellView> = cx.new(|_cx| {
            let mut shell = ShellView::new_with_sessions(seeded.clone());
            shell.app.set_config(config.clone());
            shell
        });
        let pump_view = view.clone();
        // Pump loop: poll PTYs at 20 Hz so background runs keep streaming;
        // the window repaints only when the pump reports dirtiness (fresh
        // output, exit, spawn, status/link/row change), so idle costs ~zero.
        cx.spawn(async move |cx| loop {
            cx.background_executor()
                .timer(gui::shell::PUMP_INTERVAL)
                .await;
            let alive = cx.update(|cx| {
                pump_view.update(cx, |view, cx| {
                    if view.tick() {
                        cx.notify();
                    }
                })
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
        // Root must be the window's first view: it provides the theme
        // context the sidebar/buttons read, plus dialog/notification layers.
        // Issue #34: apply the configured theme first (explicit dark/light
        // or the live OS appearance) so chrome renders in it immediately.
        cx.open_window(gui::view::window_options(), |window, cx| {
            Theme::change(
                gui::theme::theme_mode_for(startup_theme, window.appearance()),
                Some(window),
                cx,
            );
            cx.new(|cx| Root::new(view.clone(), window, cx))
        })
        .unwrap();
    });
}
