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
//! the list.

mod app;
mod embedded;
mod gui;
// Parked for the modular future (alternate providers, link-parser families,
// historic transcript attach): kept compiled and unit-tested.
#[allow(dead_code, unused_imports)]
mod parsers;
#[allow(dead_code, unused_imports)]
mod providers;
#[allow(dead_code, unused_imports)]
mod transcript;

use gpui::{AppContext, Application, Entity};
use gpui_component::{Root, Theme, ThemeMode};

use gui::shell::ShellView;

fn main() {
    Application::new().run(|cx| {
        // Native chrome components (sidebar, buttons): init once, dark to
        // match the terminal pane.
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        let view: Entity<ShellView> = cx.new(|_cx| ShellView::new());
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
        cx.open_window(gui::view::window_options(), |window, cx| {
            cx.new(|cx| Root::new(view.clone(), window, cx))
        })
        .unwrap();
    });
}
