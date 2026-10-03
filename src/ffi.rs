//! C ABI foundation over the shared core (epic #60, slice 2).
//!
//! Minimal, portable `extern "C"` seam for the future native shells: owned
//! screen snapshots (no `LiveView` lifetime crosses FFI, §5.2), integer
//! error codes + a thread-local message (§5.3), and spawn / pump / write /
//! resize / screen-text / status functions. Pure Rust, no codegen, so it
//! verifies on every OS gate. SwiftUI scaffold is slice 3.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

use crate::app::{App, ChatSession, Status};
use crate::embedded::{EmbeddedPty, SpawnKind};
use crate::launch;
use crate::parsers::registry::RegistryParser;
use crate::persist;
use crate::providers::{
    AntigravityCliProvider, ClaudeCliProvider, CodexCliProvider, MuseCliProvider,
    OpencodeCliProvider, Provider,
};

/// Integer error codes returned by every fallible `am_*` function.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmError {
    /// Success.
    Ok = 0,
    /// Child spawn failed (bad program, PTY unavailable).
    Spawn = 1,
    /// PTY input write failed.
    Io = 2,
    /// A `*const char` argument was not valid UTF-8.
    Utf8 = 3,
    /// A required pointer argument was null.
    Null = 4,
    /// Config persistence failed.
    Config = 5,
}

impl AmError {
    fn code(self) -> c_int {
        self as c_int
    }
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::new("").unwrap());
}

fn set_error(msg: String) {
    let clean = msg.replace('\0', "");
    let cstr = CString::new(clean).unwrap_or_default();
    LAST_ERROR.with(|slot| *slot.borrow_mut() = cstr);
}

/// 8-bit RGB triple (plain value; `#[repr(C)]` so shells can read it).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Cell style as plain values (`has_fg`/`has_bg` = terminal default when
/// false; the `Option` lives on the Rust side only).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmStyle {
    pub has_fg: bool,
    pub fg: AmRgb,
    pub has_bg: bool,
    pub bg: AmRgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl AmStyle {
    fn none() -> Self {
        Self {
            has_fg: false,
            fg: AmRgb { r: 0, g: 0, b: 0 },
            has_bg: false,
            bg: AmRgb { r: 0, g: 0, b: 0 },
            bold: false,
            italic: false,
            underline: false,
        }
    }
}

impl From<crate::embedded::SnapStyle> for AmStyle {
    fn from(s: crate::embedded::SnapStyle) -> Self {
        let mut out = Self::none();
        if let Some(fg) = s.fg {
            out.has_fg = true;
            out.fg = AmRgb {
                r: fg.0,
                g: fg.1,
                b: fg.2,
            };
        }
        if let Some(bg) = s.bg {
            out.has_bg = true;
            out.bg = AmRgb {
                r: bg.0,
                g: bg.1,
                b: bg.2,
            };
        }
        out.bold = s.bold;
        out.italic = s.italic;
        out.underline = s.underline;
        out
    }
}

/// Opaque core handle: owns the roster `App` (which owns its `Config`).
pub struct AmCore {
    app: App,
}

/// Opaque PTY handle: owns one [`EmbeddedPty`].
pub struct AmPty {
    pty: EmbeddedPty,
}

/// Build a core the way the app starts: discovered provider sessions
/// merged over persisted rows. Discovery and load both degrade to empty
/// (never fail), so this constructor is infallible barring allocation.
///
/// # Safety
/// No arguments; always safe to call. Free with [`am_core_free`].
#[no_mangle]
pub unsafe extern "C" fn am_core_new() -> *mut AmCore {
    let mut discovered = MuseCliProvider::new(
        MuseCliProvider::default_store_root(),
        Box::new(RegistryParser::default()),
    )
    .discover_sessions();
    // Same merge as the gpui shell startup: opencode, claude, codex, and
    // antigravity sessions seed alongside muse sessions (unreachable
    // stores/CLIs degrade to empty).
    discovered.extend(
        OpencodeCliProvider::with_default_program(Box::new(RegistryParser::default()))
            .discover_sessions(),
    );
    discovered.extend(
        ClaudeCliProvider::new(
            ClaudeCliProvider::default_store_root(),
            Box::new(RegistryParser::default()),
        )
        .discover_sessions(),
    );
    discovered.extend(
        CodexCliProvider::new(
            CodexCliProvider::default_store_root(),
            Box::new(RegistryParser::default()),
        )
        .discover_sessions(),
    );
    discovered.extend(
        AntigravityCliProvider::new(
            AntigravityCliProvider::default_store_root(),
            Box::new(RegistryParser::default()),
        )
        .discover_sessions(),
    );
    let persisted = persist::load_sessions();
    let app = App::new(persist::merge_sessions(discovered, persisted));
    Box::into_raw(Box::new(AmCore { app }))
}

/// Free a core created by [`am_core_new`]. Null is a no-op.
///
/// # Safety
/// `core` must be null or a pointer from [`am_core_new`], used at most once.
#[no_mangle]
pub unsafe extern "C" fn am_core_free(core: *mut AmCore) {
    if !core.is_null() {
        drop(Box::from_raw(core));
    }
}

/// Persist the core config. Returns an [`AmError`] code.
///
/// # Safety
/// `core` must be null or a live pointer from [`am_core_new`].
#[no_mangle]
pub unsafe extern "C" fn am_core_save(core: *const AmCore) -> c_int {
    if core.is_null() {
        set_error("am_core_save: null core".to_string());
        return AmError::Null.code();
    }
    match (*core).app.save_config() {
        Ok(()) => AmError::Ok.code(),
        Err(e) => {
            set_error(format!("am_core_save: {e:#}"));
            AmError::Config.code()
        }
    }
}

/// Spawn a fresh session PTY (`muse` + configured extra flags) of
/// `cols` x `rows`. `cwd` is null (inherit) or a NUL-terminated UTF-8
/// path; the new handle is written to `*out`. Returns an [`AmError`] code.
///
/// # Safety
/// `core`/`out` must be non-null live pointers; `cwd` must be null or a
/// valid NUL-terminated C string. Free the handle with [`am_pty_free`].
#[no_mangle]
pub unsafe extern "C" fn am_spawn(
    core: *const AmCore,
    out: *mut *mut AmPty,
    cwd: *const c_char,
    cols: u16,
    rows: u16,
) -> c_int {
    if core.is_null() || out.is_null() {
        set_error("am_spawn: null core or out".to_string());
        return AmError::Null.code();
    }
    let (program, args) = (*core).app.spawn_command_for(&SpawnKind::New);
    spawn_into(out, &program, &args, cwd, cols, rows)
}

/// Spawn a 2D-launch session PTY (folder × CLI + yolo) of `cols` x `rows`.
/// `cli` names the harness id (`muse`, `claude`, …; null/empty repeats the
/// last-used/default resolution), `cwd` is null (inherit) or a path,
/// `yolo` is tri-state: >0 forces the canonical yolo flag on for this
/// spawn only, <0 forces it off, 0 follows the per-agent config default.
/// Returns an [`AmError`] code; the handle lands in `*out`.
///
/// # Safety
/// `core`/`out` must be non-null live pointers; `cli`/`cwd` must be null
/// or valid NUL-terminated C strings. Free the handle with [`am_pty_free`].
#[no_mangle]
pub unsafe extern "C" fn am_spawn_launch(
    core: *const AmCore,
    out: *mut *mut AmPty,
    cli: *const c_char,
    cwd: *const c_char,
    yolo: c_int,
    cols: u16,
    rows: u16,
) -> c_int {
    if core.is_null() || out.is_null() {
        set_error("am_spawn_launch: null core or out".to_string());
        return AmError::Null.code();
    }
    let cli_str = if cli.is_null() {
        None
    } else {
        match CStr::from_ptr(cli).to_str() {
            Ok(s) => Some(s),
            Err(_) => {
                set_error("am_spawn_launch: cli is not valid UTF-8".to_string());
                return AmError::Utf8.code();
            }
        }
    };
    let _cwd_str = if cwd.is_null() {
        None
    } else {
        match CStr::from_ptr(cwd).to_str() {
            Ok(s) => Some(s),
            Err(_) => {
                set_error("am_spawn_launch: cwd is not valid UTF-8".to_string());
                return AmError::Utf8.code();
            }
        }
    };
    let app = &(*core).app;
    let resolved_cli = launch::resolve_effective_cli(
        cli_str.filter(|s| !s.is_empty()),
        app.config_last_cli(),
        app.config_default_cli(),
        &launch::detect_available_clis(),
    );
    let harness = crate::embedded::Harness::from_id(&resolved_cli);
    // Tri-state yolo (mirrors `launch::YoloChoice`): positive forces on,
    // negative forces off, zero follows the per-agent config default.
    let yolo_on = if yolo > 0 {
        true
    } else if yolo < 0 {
        false
    } else {
        app.config_yolo_default(harness.id())
    };
    let (program, mut args) = harness.new_command();
    args.extend(app.config_extra_args(&program));
    if yolo_on {
        if let Some(flag) = harness.yolo_flag() {
            if !args.iter().any(|a| a == flag) {
                args.push(flag.to_string());
            }
        }
    }
    spawn_into(out, &program, &args, cwd, cols, rows)
}

/// Owned harness id of the effective CLI for `cli` (2D launch): the
/// explicit id when non-empty, else the core's last-used / configured /
/// autodetected resolution (same rule as [`am_spawn_launch`]). Lets
/// shells label a repeat-last spawn before starting it. Null `cli`
/// means "resolve the default". Free with [`am_screen_text_free`].
///
/// # Safety
/// `core` must be null or a live pointer from [`am_core_new`]; `cli`
/// must be null or a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn am_effective_cli(core: *const AmCore, cli: *const c_char) -> *mut c_char {
    let explicit = if cli.is_null() {
        None
    } else {
        match CStr::from_ptr(cli).to_str() {
            Ok(s) => Some(s),
            Err(_) => {
                set_error("am_effective_cli: cli is not valid UTF-8".to_string());
                return std::ptr::null_mut();
            }
        }
    };
    let catalog = launch::detect_available_clis();
    let (last, default) = if core.is_null() {
        (None, None)
    } else {
        (
            (*core).app.config_last_cli(),
            (*core).app.config_default_cli(),
        )
    };
    let resolved =
        launch::resolve_effective_cli(explicit.filter(|s| !s.is_empty()), last, default, &catalog);
    match CString::new(resolved) {
        Ok(s) => s.into_raw(),
        Err(_) => {
            set_error("am_effective_cli: id contains NUL".to_string());
            std::ptr::null_mut()
        }
    }
}

/// Owned JSON of the autodetected CLI catalog (2D launch):
/// `[{"id","program","path"|null,"available"}]` in
/// [`launch::SUPPORTED_CLIS`] order. Never null on allocation success;
/// free with [`am_screen_text_free`].
///
/// # Safety
/// Always safe: allocates a fresh string, takes no handles.
#[no_mangle]
pub unsafe extern "C" fn am_clis_json() -> *mut c_char {
    let clis = launch::detect_available_clis();
    match serde_json::to_string(&clis) {
        Ok(doc) => match CString::new(doc) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_error("am_clis_json: catalog contains NUL".to_string());
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            set_error(format!("am_clis_json: {e:#}"));
            std::ptr::null_mut()
        }
    }
}

/// Owned JSON of the persisted folder recents (2D launch): a string array,
/// MRU-first. Null core yields an empty list, never UB; free with
/// [`am_screen_text_free`].
///
/// # Safety
/// `core` must be null or a live pointer from [`am_core_new`].
#[no_mangle]
pub unsafe extern "C" fn am_recent_json(core: *const AmCore) -> *mut c_char {
    let recents: &[String] = if core.is_null() {
        &[]
    } else {
        (*core).app.config_recents()
    };
    match serde_json::to_string(recents) {
        Ok(doc) => match CString::new(doc) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_error("am_recent_json: recents contain NUL".to_string());
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            set_error(format!("am_recent_json: {e:#}"));
            std::ptr::null_mut()
        }
    }
}

/// Record a confirmed launch (2D launch): refreshes last-used CLI + folder
/// MRU in the core config. Null core is a null error; null/empty `cli`
/// keeps the previous CLI; null `cwd` records no folder.
///
/// # Safety
/// `core` must be non-null and live; `cli`/`cwd` must be null or valid
/// NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn am_note_launch(
    core: *mut AmCore,
    cli: *const c_char,
    cwd: *const c_char,
) -> c_int {
    if core.is_null() {
        set_error("am_note_launch: null core".to_string());
        return AmError::Null.code();
    }
    let cli_str = if cli.is_null() {
        None
    } else {
        match CStr::from_ptr(cli).to_str() {
            Ok(s) => Some(s),
            Err(_) => {
                set_error("am_note_launch: cli is not valid UTF-8".to_string());
                return AmError::Utf8.code();
            }
        }
    };
    let cwd_str = if cwd.is_null() {
        None
    } else {
        match CStr::from_ptr(cwd).to_str() {
            Ok(s) => Some(s),
            Err(_) => {
                set_error("am_note_launch: cwd is not valid UTF-8".to_string());
                return AmError::Utf8.code();
            }
        }
    };
    (*core)
        .app
        .note_launch(cli_str.filter(|s| !s.is_empty()).unwrap_or(""), cwd_str);
    AmError::Ok.code()
}

/// Shared spawn tail for [`am_spawn`] (and the test-only argv variant
/// below): parse `cwd`, spawn, box the handle. One seam so the public
/// success path and the hermetic tests share every line after argv.
unsafe fn spawn_into(
    out: *mut *mut AmPty,
    program: &str,
    args: &[String],
    cwd: *const c_char,
    cols: u16,
    rows: u16,
) -> c_int {
    let cwd_path;
    let cwd_opt = if cwd.is_null() {
        None
    } else {
        match CStr::from_ptr(cwd).to_str() {
            Ok(s) => {
                cwd_path = std::path::PathBuf::from(s);
                Some(cwd_path.as_path())
            }
            Err(_) => {
                set_error("am_spawn: cwd is not valid UTF-8".to_string());
                return AmError::Utf8.code();
            }
        }
    };
    match EmbeddedPty::spawn_with_cwd(program, args, cols, rows, cwd_opt) {
        Ok(pty) => {
            *out = Box::into_raw(Box::new(AmPty { pty }));
            AmError::Ok.code()
        }
        Err(e) => {
            set_error(format!("am_spawn: {e:#}"));
            AmError::Spawn.code()
        }
    }
}

/// Feed queued output into the emulator. Returns true when new output
/// arrived or the child newly exited (the only dirty gate the shell
/// needs to repaint). Null is false, never UB.
///
/// # Safety
/// `pty` must be null or a live pointer from [`am_spawn`].
#[no_mangle]
pub unsafe extern "C" fn am_pump(pty: *mut AmPty) -> bool {
    if pty.is_null() {
        return false;
    }
    (*pty).pty.pump()
}

/// Forward raw bytes (already key-encoded by the shell) to the child.
/// Returns an [`AmError`] code.
///
/// # Safety
/// `pty` must be non-null and live; `data` must point to `len` readable
/// bytes when `len > 0` (null `data` with `len == 0` is a no-op success).
#[no_mangle]
pub unsafe extern "C" fn am_write(pty: *mut AmPty, data: *const u8, len: usize) -> c_int {
    if pty.is_null() {
        set_error("am_write: null pty".to_string());
        return AmError::Null.code();
    }
    if data.is_null() {
        if len == 0 {
            return AmError::Ok.code();
        }
        set_error("am_write: null data".to_string());
        return AmError::Null.code();
    }
    let bytes = std::slice::from_raw_parts(data, len);
    match (*pty).pty.write_input(bytes) {
        Ok(()) => AmError::Ok.code(),
        Err(e) => {
            set_error(format!("am_write: {e:#}"));
            AmError::Io.code()
        }
    }
}

/// Resize the PTY and the emulator grid. Null is a no-op.
///
/// # Safety
/// `pty` must be null or a live pointer from [`am_spawn`].
#[no_mangle]
pub unsafe extern "C" fn am_resize(pty: *mut AmPty, cols: u16, rows: u16) {
    if !pty.is_null() {
        (*pty).pty.resize(cols, rows);
    }
}

/// Owned UTF-8 snapshot of the emulated screen. Returns null on null
/// handle; otherwise a freshly allocated string the caller frees with
/// [`am_screen_text_free`].
///
/// # Safety
/// `pty` must be null or a live pointer from [`am_spawn`].
#[no_mangle]
pub unsafe extern "C" fn am_screen_text(pty: *const AmPty) -> *mut c_char {
    if pty.is_null() {
        return std::ptr::null_mut();
    }
    let text = (*pty).pty.snapshot_text();
    match CString::new(text) {
        Ok(s) => s.into_raw(),
        Err(_) => {
            set_error("am_screen_text: snapshot contains NUL".to_string());
            std::ptr::null_mut()
        }
    }
}

/// Free a string returned by [`am_screen_text`] or [`am_spans_json`].
/// Null is a no-op.
///
/// # Safety
/// `s` must be null or a pointer from [`am_screen_text`]/[`am_spans_json`],
/// used at most once.
#[no_mangle]
pub unsafe extern "C" fn am_screen_text_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Owned styled spans of the emulated screen as JSON:
/// `[[{"text":..,"fg":[r,g,b]|null,"bg":..,"bold":..,"italic":..,
/// "underline":..}]]` (one array per grid row). Returns null on null
/// handle or (unreachable in practice) JSON failure; free with
/// [`am_screen_text_free`].
///
/// # Safety
/// `pty` must be null or a live pointer from [`am_spawn`].
#[no_mangle]
pub unsafe extern "C" fn am_spans_json(pty: *const AmPty) -> *mut c_char {
    if pty.is_null() {
        return std::ptr::null_mut();
    }
    let rows = (*pty).pty.snapshot_spans();
    let json_rows: Vec<Vec<serde_json::Value>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|span| {
                    let style = AmStyle::from(span.style);
                    serde_json::json!({
                        "text": span.text,
                        "fg": if style.has_fg { serde_json::json!([style.fg.r, style.fg.g, style.fg.b]) } else { serde_json::Value::Null },
                        "bg": if style.has_bg { serde_json::json!([style.bg.r, style.bg.g, style.bg.b]) } else { serde_json::Value::Null },
                        "bold": style.bold,
                        "italic": style.italic,
                        "underline": style.underline,
                    })
                })
                .collect()
        })
        .collect();
    let doc = serde_json::Value::Array(
        json_rows
            .into_iter()
            .map(serde_json::Value::Array)
            .collect(),
    );
    match CString::new(doc.to_string()) {
        Ok(s) => s.into_raw(),
        Err(_) => {
            set_error("am_spans_json: snapshot contains NUL".to_string());
            std::ptr::null_mut()
        }
    }
}

/// Roster status of row `row`: 0 = Attention, 1 = Idle, 2 = Working.
/// Returns -1 on null handle, -2 when `row` is out of bounds.
///
/// # Safety
/// `core` must be null or a live pointer from [`am_core_new`].
#[no_mangle]
pub unsafe extern "C" fn am_status(core: *const AmCore, row: usize) -> c_int {
    if core.is_null() {
        set_error("am_status: null core".to_string());
        return -1;
    }
    let sessions: &[ChatSession] = &(*core).app.sessions;
    match sessions.get(row) {
        Some(session) => match session.status {
            Status::Attention => 0,
            Status::Idle => 1,
            Status::Working => 2,
        },
        None => -2,
    }
}

/// Number of roster rows in the core. Null core yields 0 (never UB).
///
/// # Safety
/// `core` must be null or a live pointer from [`am_core_new`].
#[no_mangle]
pub unsafe extern "C" fn am_session_count(core: *const AmCore) -> usize {
    if core.is_null() {
        return 0;
    }
    (*core).app.sessions.len()
}

/// Owned JSON of roster row `row` (a serialized [`ChatSession`]; the
/// documented roster crossing from the seam sketch). Returns null on
/// null handle, out-of-bounds row, or (unreachable in practice) JSON
/// failure; free with [`am_screen_text_free`].
///
/// # Safety
/// `core` must be null or a live pointer from [`am_core_new`].
#[no_mangle]
pub unsafe extern "C" fn am_session_json(core: *const AmCore, row: usize) -> *mut c_char {
    if core.is_null() {
        set_error("am_session_json: null core".to_string());
        return std::ptr::null_mut();
    }
    let sessions: &[ChatSession] = &(*core).app.sessions;
    match sessions.get(row) {
        Some(session) => match serde_json::to_string(session) {
            Ok(doc) => match CString::new(doc) {
                Ok(s) => s.into_raw(),
                Err(_) => {
                    set_error("am_session_json: row contains NUL".to_string());
                    std::ptr::null_mut()
                }
            },
            Err(e) => {
                set_error(format!("am_session_json: {e:#}"));
                std::ptr::null_mut()
            }
        },
        None => {
            set_error(format!("am_session_json: row {row} out of bounds"));
            std::ptr::null_mut()
        }
    }
}

/// Snapshot-to-stream feed reconciler for C shells (Windows + Linux
/// parity): computes the text that advances a view showing `old_text` to
/// also show `new_text`, or null when the view is already current. Either
/// argument may be null (treated as ""). The result is freshly allocated;
/// free it with [`am_screen_text_free`].
///
/// This is the shared [`crate::shell_shared::feed_delta`] (append-only
/// suffix hot path, scroll overlap, clear-and-replay redraw, CRLF
/// normalization), replacing the per-shell `feed.c` ports.
///
/// # Safety
/// `old_text`/`new_text` must be null or valid NUL-terminated UTF-8
/// C strings.
#[no_mangle]
pub unsafe extern "C" fn am_feed_delta(
    old_text: *const c_char,
    new_text: *const c_char,
) -> *mut c_char {
    let old = c_str_or_empty(old_text);
    let new = c_str_or_empty(new_text);
    let (Some(old), Some(new)) = (old, new) else {
        set_error("am_feed_delta: argument is not valid UTF-8".to_string());
        return std::ptr::null_mut();
    };
    match crate::shell_shared::feed_delta(old, new) {
        Some(feed) => match CString::new(feed) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_error("am_feed_delta: feed contains NUL".to_string());
                std::ptr::null_mut()
            }
        },
        None => std::ptr::null_mut(),
    }
}

/// Decode a nullable C string to `Some(str)` (null becomes `Some("")`);
/// `None` when the bytes are not valid UTF-8. Shared by the nullable
/// string arguments below so invalid UTF-8 degrades uniformly.
unsafe fn c_str_or_empty(ptr: *const c_char) -> Option<&'static str> {
    if ptr.is_null() {
        return Some("");
    }
    // Extend the borrow to 'static: the pointer is only read during this
    // call and never retained, matching every other getter in this file.
    CStr::from_ptr(ptr)
        .to_str()
        .ok()
        .map(|s| unsafe { &*(s as *const str) })
}

/// True (1) when a roster row with this title/project/id passes the
/// sidebar `query` (case-insensitive substring; blank query passes
/// everything), else false (0). Null pointers mean empty strings; invalid
/// UTF-8 in any argument reports false and records a message.
///
/// # Safety
/// Each argument must be null or a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn am_roster_matches(
    title: *const c_char,
    project: *const c_char,
    id: *const c_char,
    query: *const c_char,
) -> c_int {
    let (Some(title), Some(project), Some(id), Some(query)) = (
        c_str_or_empty(title),
        c_str_or_empty(project),
        c_str_or_empty(id),
        c_str_or_empty(query),
    ) else {
        set_error("am_roster_matches: argument is not valid UTF-8".to_string());
        return 0;
    };
    i32::from(crate::shell_shared::roster_matches(
        title, project, id, query,
    ))
}

/// Owned glanceable age label for `then_secs` (unix seconds) relative to
/// `now_secs`: `just now` / `Nm ago` / `Nh ago` / `Nd ago`. Free with
/// [`am_screen_text_free`].
///
/// # Safety
/// Always safe: pure computation, no pointers read.
#[no_mangle]
pub unsafe extern "C" fn am_relative_age(now_secs: i64, then_secs: i64) -> *mut c_char {
    match CString::new(crate::shell_shared::relative_age(now_secs, then_secs)) {
        Ok(s) => s.into_raw(),
        Err(_) => {
            set_error("am_relative_age: label contains NUL".to_string());
            std::ptr::null_mut()
        }
    }
}

/// Total link count (PR + related) of roster row `row`, or -1 on null
/// handle / out-of-bounds row. Lets shells show the same link badge
/// without parsing `pr_links` / `related_links` themselves.
///
/// # Safety
/// `core` must be null or a live pointer from [`am_core_new`].
#[no_mangle]
pub unsafe extern "C" fn am_link_count(core: *const AmCore, row: usize) -> c_int {
    if core.is_null() {
        set_error("am_link_count: null core".to_string());
        return -1;
    }
    let sessions: &[ChatSession] = &(*core).app.sessions;
    match sessions.get(row) {
        Some(session) => (session.pr_links.len() + session.related_links.len()) as c_int,
        None => {
            set_error(format!("am_link_count: row {row} out of bounds"));
            -1
        }
    }
}

/// Unix seconds of `last_active` for roster row `row`, or -1 on null
/// handle / out-of-bounds row. Shells feed it to [`am_relative_age`]
/// with their own clock, so all shells render the same relative label.
///
/// # Safety
/// `core` must be null or a live pointer from [`am_core_new`].
#[no_mangle]
pub unsafe extern "C" fn am_last_active(core: *const AmCore, row: usize) -> i64 {
    if core.is_null() {
        set_error("am_last_active: null core".to_string());
        return -1;
    }
    let sessions: &[ChatSession] = &(*core).app.sessions;
    match sessions.get(row) {
        Some(session) => session.last_active,
        None => {
            set_error(format!("am_last_active: row {row} out of bounds"));
            -1
        }
    }
}

/// Last error message for this thread (UTF-8, NUL-terminated). Never
/// null; valid until the next failing `am_*` call on this thread.
///
/// # Safety
/// Always safe: returns a thread-local pointer, never transfers ownership.
#[no_mangle]
pub unsafe extern "C" fn am_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

/// Free a PTY created by [`am_spawn`] (reaps the child). Null is a no-op.
///
/// # Safety
/// `pty` must be null or a pointer from [`am_spawn`], used at most once.
#[no_mangle]
pub unsafe extern "C" fn am_pty_free(pty: *mut AmPty) {
    if !pty.is_null() {
        drop(Box::from_raw(pty));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn pump_until_text(pty: *mut AmPty, needle: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            unsafe {
                am_pump(pty);
                let raw = am_screen_text(pty);
                if raw.is_null() {
                    continue;
                }
                let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
                am_screen_text_free(raw);
                if text.contains(needle) {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn error_codes_round_trip() {
        assert_eq!(AmError::Ok as c_int, 0);
        assert_eq!(AmError::Spawn as c_int, 1);
        assert_eq!(AmError::Io as c_int, 2);
        assert_eq!(AmError::Utf8 as c_int, 3);
        assert_eq!(AmError::Null as c_int, 4);
        assert_eq!(AmError::Config as c_int, 5);
    }

    #[test]
    fn null_ptrs_return_errors_never_ub() {
        unsafe {
            assert_eq!(
                am_spawn(
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    80,
                    24
                ),
                AmError::Null.code()
            );
            // Null core with a live out pointer is still a null error.
            let mut out: *mut AmPty = std::ptr::null_mut();
            assert_eq!(
                am_spawn(std::ptr::null(), &mut out, std::ptr::null(), 80, 24),
                AmError::Null.code()
            );
            assert!(out.is_null());
            assert!(!am_pump(std::ptr::null_mut()));
            assert_eq!(
                am_write(std::ptr::null_mut(), std::ptr::null(), 0),
                AmError::Null.code()
            );
            am_resize(std::ptr::null_mut(), 80, 24);
            assert!(am_screen_text(std::ptr::null()).is_null());
            assert!(am_spans_json(std::ptr::null()).is_null());
            assert_eq!(am_status(std::ptr::null(), 0), -1);
            assert_eq!(am_core_save(std::ptr::null()), AmError::Null.code());
            am_core_free(std::ptr::null_mut());
            am_pty_free(std::ptr::null_mut());
            am_screen_text_free(std::ptr::null_mut());
            // Every failing call above recorded a message; the pointer is
            // always valid and non-null.
            let msg = am_last_error();
            assert!(!msg.is_null());
            assert!(!CStr::from_ptr(msg).to_bytes().is_empty());
        }
    }

    #[test]
    fn snapshot_owns_data_after_pty_drop() {
        // §5.2 core claim: the snapshot outlives (and is unaffected by
        // mutating/dropping) the PTY it was copied from.
        unsafe {
            let core = am_core_new();
            assert!(!core.is_null());
            // Spawn `echo` directly: deterministic without a real `muse`.
            let mut pty: *mut AmPty = std::ptr::null_mut();
            let prog = CString::new("echo").unwrap();
            let arg = CString::new("ffi-owned-snap").unwrap();
            let argv = [prog.as_ptr(), arg.as_ptr()];
            let rc = am_spawn_argv(core, &mut pty, argv.as_ptr(), 2, std::ptr::null(), 80, 24);
            assert_eq!(rc, AmError::Ok.code());
            assert!(!pty.is_null());
            assert!(pump_until_text(
                pty,
                "ffi-owned-snap",
                Duration::from_secs(5)
            ));
            let raw = am_screen_text(pty);
            assert!(!raw.is_null());
            let owned = CStr::from_ptr(raw).to_string_lossy().into_owned();
            am_screen_text_free(raw);
            // Mutate then drop the PTY: the owned copy must not change.
            am_resize(pty, 100, 30);
            am_pump(pty);
            am_pty_free(pty);
            assert!(owned.contains("ffi-owned-snap"));
            // Spans variant carries the same text (JSON-escaped or plain).
            let core2 = am_core_new();
            let mut pty2: *mut AmPty = std::ptr::null_mut();
            assert_eq!(
                am_spawn_argv(core2, &mut pty2, argv.as_ptr(), 2, std::ptr::null(), 80, 24),
                AmError::Ok.code()
            );
            assert!(pump_until_text(
                pty2,
                "ffi-owned-snap",
                Duration::from_secs(5)
            ));
            let spans_raw = am_spans_json(pty2);
            assert!(!spans_raw.is_null());
            let spans = CStr::from_ptr(spans_raw).to_string_lossy().into_owned();
            am_screen_text_free(spans_raw);
            am_pty_free(pty2);
            assert!(spans.contains("ffi-owned-snap"));
            am_core_free(core);
            am_core_free(core2);
        }
    }

    // Unix-only like the fake-`muse` test above: `printf` is a POSIX
    // utility, not guaranteed on Windows.
    #[cfg(unix)]
    #[test]
    fn spans_json_preserves_sgr_red() {
        // Issue #70 falsifiable at the core edge: a child emitting ANSI
        // colors (`printf '\e[31mred\e[0m'`) must surface a red span, which
        // is what the Swift pump re-emits as SGR for the terminal view.
        unsafe {
            let core = am_core_new();
            assert!(!core.is_null());
            let mut pty: *mut AmPty = std::ptr::null_mut();
            let prog = CString::new("printf").unwrap();
            // POSIX octal escapes in the format string: the child emits
            // real SGR bytes around `red`.
            let arg = CString::new("\\033[31mred\\033[0m\\n").unwrap();
            let argv = [prog.as_ptr(), arg.as_ptr()];
            let rc = am_spawn_argv(core, &mut pty, argv.as_ptr(), 2, std::ptr::null(), 80, 24);
            assert_eq!(rc, AmError::Ok.code());
            assert!(!pty.is_null());
            assert!(pump_until_text(pty, "red", Duration::from_secs(5)));
            // The plain snapshot strips SGR (the #70 root cause on its own).
            let raw = am_screen_text(pty);
            assert!(!raw.is_null());
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            am_screen_text_free(raw);
            assert!(text.contains("red"));
            assert!(!text.contains('\x1b'));
            // The spans snapshot keeps the red style (palette 205,0,0).
            let spans_raw = am_spans_json(pty);
            assert!(!spans_raw.is_null());
            let spans = CStr::from_ptr(spans_raw).to_string_lossy().into_owned();
            am_screen_text_free(spans_raw);
            am_pty_free(pty);
            am_core_free(core);
            assert!(spans.contains("red"), "spans carry text: {spans}");
            assert!(
                spans.contains("[205,0,0]"),
                "red SGR maps to palette red: {spans}"
            );
        }
    }

    #[test]
    fn spawn_failure_reports_spawn_code() {
        unsafe {
            let core = am_core_new();
            assert!(!core.is_null());
            let mut pty: *mut AmPty = std::ptr::null_mut();
            let prog = CString::new("definitely-not-a-real-binary-xyz").unwrap();
            let argv = [prog.as_ptr()];
            let rc = am_spawn_argv(core, &mut pty, argv.as_ptr(), 1, std::ptr::null(), 80, 24);
            assert_eq!(rc, AmError::Spawn.code());
            assert!(pty.is_null());
            let msg = CStr::from_ptr(am_last_error())
                .to_string_lossy()
                .into_owned();
            assert!(msg.contains("definitely-not-a-real-binary-xyz"));
            assert_eq!(am_status(core, usize::MAX), -2);
            am_core_free(core);
        }
    }

    #[test]
    fn status_codes_map_roster_status() {
        use crate::app::{App, ChatSession, HARNESS_MUSE};
        fn row(id: &str, status: Status) -> ChatSession {
            ChatSession {
                id: id.into(),
                title: id.into(),
                project: "proj".into(),
                status,
                harness: HARNESS_MUSE.into(),
                last_active: 0,
                pr_links: vec![],
                related_links: vec![],
                links_truncated: false,
                transcript: vec![],
                transcript_truncated: false,
                title_locked: true,
                pending_input: String::new(),
                provider_session_id: None,
                cwd: None,
            }
        }
        let core = AmCore {
            app: App::new(vec![
                row("a", Status::Attention),
                row("b", Status::Idle),
                row("c", Status::Working),
            ]),
        };
        unsafe {
            assert_eq!(am_status(&core as *const AmCore, 0), 0);
            assert_eq!(am_status(&core as *const AmCore, 1), 1);
            assert_eq!(am_status(&core as *const AmCore, 2), 2);
            assert_eq!(am_status(&core as *const AmCore, 3), -2);
            // Null handle reports -1 and records a message (never UB).
            assert_eq!(am_status(std::ptr::null(), 0), -1);
            let msg = CStr::from_ptr(am_last_error())
                .to_string_lossy()
                .into_owned();
            assert!(msg.contains("am_status"), "last_error: {msg:?}");
        }
    }

    /// Restores a process env var on drop (tests share one process).
    struct EnvGuard {
        key: &'static str,
        old: Option<String>,
    }
    impl EnvGuard {
        fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
            let old = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, old }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// Serializes the PATH-mutating hermetic spawn tests: `cargo test`
    /// runs threads in one process, so two tests prepending different
    /// fake-CLI dirs would clobber each other's PATH mid-spawn (a lost
    /// fake reads as a spawn failure). Hold this across the whole
    /// prepend-spawn-restore window.
    #[cfg(unix)]
    static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn unique_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "agent-manager-ffi61-{tag}-{}-{}",
            std::process::id(),
            nanos
        ))
    }

    // Unix-only: the fake is an extensionless shell script, which
    // CreateProcess cannot execute. The Windows counterpart is the
    // hermetic smoke-live step of the windows CI job (issue #64):
    // tests/smoke_live.ps1 compiles tests/fake_muse.c to muse.exe and
    // runs am-win-smoke --smoke-live against it.
    #[cfg(unix)]
    #[test]
    fn public_spawn_success_path() {
        // Issue #61: the review flagged the public `am_spawn` (which runs
        // the real `muse` command) as success-untested. A fake `muse`
        // earlier on PATH exercises the exact public entry point,
        // hermetically: no real agent, no live config.
        let dir = unique_dir("muse");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("muse"),
            "#!/bin/sh\necho fake-muse-ready-61\nsleep 30\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("muse"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let old_path = std::env::var("PATH").unwrap_or_default();
        let joined = format!("{}:{}", dir.display(), old_path);
        let _lock = PATH_LOCK.lock().unwrap();
        let _path = EnvGuard::set("PATH", std::ffi::OsStr::new(&joined));
        unsafe {
            let core = am_core_new();
            assert!(!core.is_null());
            let mut pty: *mut AmPty = std::ptr::null_mut();
            let rc = am_spawn(core, &mut pty, std::ptr::null(), 80, 24);
            assert_eq!(rc, AmError::Ok.code());
            assert!(!pty.is_null());
            assert!(pump_until_text(
                pty,
                "fake-muse-ready-61",
                Duration::from_secs(10)
            ));
            am_pty_free(pty);
            am_core_free(core);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn core_save_round_trips_to_scoped_config() {
        // Issue #61: `am_core_save` was deliberately untested (live user
        // config). `$AGENT_MANAGER_CONFIG` scopes it to a temp file, so
        // the success path is covered without touching real config.
        let dir = unique_dir("cfg");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.json");
        let _scoped = EnvGuard::set("AGENT_MANAGER_CONFIG", file.as_os_str());
        unsafe {
            let core = am_core_new();
            assert!(!core.is_null());
            assert_eq!(am_core_save(core), AmError::Ok.code());
            am_core_free(core);
        }
        let text = std::fs::read_to_string(&file).expect("scoped config written");
        let _: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        // Failure path: the scoped path is a directory, so the write
        // fails and the Config code (plus message) is reported.
        let sub = dir.join("adir");
        std::fs::create_dir_all(&sub).unwrap();
        let _scoped2 = EnvGuard::set("AGENT_MANAGER_CONFIG", sub.as_os_str());
        unsafe {
            let core = am_core_new();
            assert!(!core.is_null());
            assert_eq!(am_core_save(core), AmError::Config.code());
            let msg = CStr::from_ptr(am_last_error())
                .to_string_lossy()
                .into_owned();
            assert!(msg.contains("am_core_save"), "last_error: {msg:?}");
            am_core_free(core);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_and_cwd_edge_cases_report_codes() {
        unsafe {
            let core = am_core_new();
            assert!(!core.is_null());
            let mut pty: *mut AmPty = std::ptr::null_mut();
            let prog = CString::new("sleep").unwrap();
            let arg = CString::new("30").unwrap();
            let argv = [prog.as_ptr(), arg.as_ptr()];
            let rc = am_spawn_argv(core, &mut pty, argv.as_ptr(), 2, std::ptr::null(), 80, 24);
            assert_eq!(rc, AmError::Ok.code());
            // Null data with len > 0 on a live handle is a Null error.
            assert_eq!(am_write(pty, std::ptr::null(), 1), AmError::Null.code());
            let msg = CStr::from_ptr(am_last_error())
                .to_string_lossy()
                .into_owned();
            assert!(msg.contains("am_write"), "last_error: {msg:?}");
            // Non-UTF8 cwd is a Utf8 error and spawns nothing.
            let bad: [u8; 2] = [0xFF, 0x00];
            let mut out2: *mut AmPty = std::ptr::null_mut();
            assert_eq!(
                am_spawn_argv(
                    core,
                    &mut out2,
                    argv.as_ptr(),
                    2,
                    bad.as_ptr() as *const c_char,
                    80,
                    24
                ),
                AmError::Utf8.code()
            );
            assert!(out2.is_null());
            let msg = CStr::from_ptr(am_last_error())
                .to_string_lossy()
                .into_owned();
            assert!(msg.contains("UTF-8"), "last_error: {msg:?}");
            am_pty_free(pty);
            am_core_free(core);
        }
    }

    #[test]
    fn session_count_and_json_round_trip() {
        use crate::app::{App, ChatSession, HARNESS_MUSE};
        fn row(id: &str, status: Status) -> ChatSession {
            ChatSession {
                id: id.into(),
                title: id.into(),
                project: "proj".into(),
                status,
                harness: HARNESS_MUSE.into(),
                last_active: 0,
                pr_links: vec![],
                related_links: vec![],
                links_truncated: false,
                transcript: vec![],
                transcript_truncated: false,
                title_locked: true,
                pending_input: String::new(),
                provider_session_id: None,
                cwd: None,
            }
        }
        let core = AmCore {
            app: App::new(vec![row("a", Status::Attention), row("b", Status::Idle)]),
        };
        unsafe {
            assert_eq!(am_session_count(&core as *const AmCore), 2);
            assert_eq!(am_session_count(std::ptr::null()), 0);
            let raw = am_session_json(&core as *const AmCore, 0);
            assert!(!raw.is_null());
            let text = CStr::from_ptr(raw).to_str().unwrap().to_owned();
            am_screen_text_free(raw);
            let doc: serde_json::Value =
                serde_json::from_str(&text).expect("row serializes as JSON");
            assert_eq!(doc["id"], "a");
            assert_eq!(doc["title"], "a");
            assert_eq!(doc["project"], "proj");
            assert_eq!(doc["status"], "Attention");
            // Out-of-bounds and null handles report null with a message.
            assert!(am_session_json(&core as *const AmCore, 7).is_null());
            let msg = CStr::from_ptr(am_last_error())
                .to_string_lossy()
                .into_owned();
            assert!(msg.contains("am_session_json"), "last_error: {msg:?}");
            assert!(am_session_json(std::ptr::null(), 0).is_null());
        }
    }

    #[test]
    fn launch_abi_lists_catalog_recents_and_spawns() {
        // 2D launch: the catalog lists every supported CLI in order, the
        // recents start empty on null, and a fake-CLI spawn through the
        // public launch entry resolves flags without touching live state.
        use crate::launch::SUPPORTED_CLIS;
        unsafe {
            let raw = am_clis_json();
            assert!(!raw.is_null());
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            am_screen_text_free(raw);
            let docs: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
            let ids: Vec<&str> = docs.iter().map(|d| d["id"].as_str().unwrap()).collect();
            assert_eq!(ids, SUPPORTED_CLIS);
            // Null core: empty recents, never UB.
            let recents_raw = am_recent_json(std::ptr::null());
            assert!(!recents_raw.is_null());
            let recents = CStr::from_ptr(recents_raw).to_string_lossy().into_owned();
            am_screen_text_free(recents_raw);
            assert_eq!(recents, "[]");
            // Null guards on the new entry points.
            let mut out: *mut AmPty = std::ptr::null_mut();
            assert_eq!(
                am_spawn_launch(
                    std::ptr::null(),
                    &mut out,
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    80,
                    24
                ),
                AmError::Null.code()
            );
            assert_eq!(
                am_note_launch(std::ptr::null_mut(), std::ptr::null(), std::ptr::null()),
                AmError::Null.code()
            );
        }
        // Note + recents round-trip on a live core.
        let core = AmCore {
            app: App::new(vec![]),
        };
        let core_ptr = &core as *const AmCore as *mut AmCore;
        unsafe {
            let cli = CString::new("claude").unwrap();
            let cwd = CString::new("/tmp/api").unwrap();
            assert_eq!(
                am_note_launch(core_ptr, cli.as_ptr(), cwd.as_ptr()),
                AmError::Ok.code()
            );
            let raw = am_recent_json(core_ptr);
            assert!(!raw.is_null());
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            am_screen_text_free(raw);
            assert_eq!(text, "[\"/tmp/api\"]");
            // Effective CLI: explicit wins, null resolves the default
            // (here the noted last_cli), null core still resolves.
            let explicit = CString::new("codex").unwrap();
            let raw = am_effective_cli(core_ptr, explicit.as_ptr());
            assert!(!raw.is_null());
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            am_screen_text_free(raw);
            assert_eq!(text, "codex");
            let raw = am_effective_cli(core_ptr, std::ptr::null());
            assert!(!raw.is_null());
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            am_screen_text_free(raw);
            assert_eq!(text, "claude");
            let raw = am_effective_cli(std::ptr::null(), std::ptr::null());
            assert!(!raw.is_null());
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            am_screen_text_free(raw);
            assert!(!text.is_empty());
        }
    }

    // Unix-only like `public_spawn_success_path`: the fake is a shell
    // script (CreateProcess cannot execute it; Windows coverage is the
    // smoke-live harness).
    #[cfg(unix)]
    #[test]
    fn spawn_launch_yolo_tristate_reaches_child_argv() {
        // 2D launch: the tri-state yolo int flows into the child argv —
        // force-on appends the canonical flag, force-off suppresses the
        // opted-in config default, default follows it. The fake `muse`
        // echoes its argv, so the screen proves the flag story.
        use std::time::Duration;
        let dir = unique_dir("launch-yolo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("muse"), "#!/bin/sh\necho ARGV:$*\nsleep 30\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("muse"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let old_path = std::env::var("PATH").unwrap_or_default();
        let joined = format!("{}:{}", dir.display(), old_path);
        let _lock = PATH_LOCK.lock().unwrap();
        let _path = EnvGuard::set("PATH", std::ffi::OsStr::new(&joined));
        // Opt the agent into yolo by default: default(0) and force-on(1)
        // must show `--yolo`, force-off(-1) must not.
        unsafe {
            let core = am_core_new();
            assert!(!core.is_null());
            (*core).app.config_mut().agents.insert(
                "muse".to_string(),
                crate::config::AgentConfig {
                    extra_args: Vec::new(),
                    yolo: true,
                },
            );
            for (yolo, want_flag) in [(0, true), (1, true), (-1, false)] {
                let mut pty: *mut AmPty = std::ptr::null_mut();
                let cli = CString::new("muse").unwrap();
                let rc =
                    am_spawn_launch(core, &mut pty, cli.as_ptr(), std::ptr::null(), yolo, 80, 24);
                assert_eq!(rc, AmError::Ok.code());
                assert!(!pty.is_null());
                assert!(pump_until_text(pty, "ARGV:", Duration::from_secs(10)));
                let raw = am_screen_text(pty);
                assert!(!raw.is_null());
                let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
                am_screen_text_free(raw);
                am_pty_free(pty);
                assert_eq!(
                    text.contains("--yolo"),
                    want_flag,
                    "yolo={yolo}: screen {text:?}"
                );
            }
            am_core_free(core);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_shared_feed_matches_swift_contract() {
        unsafe fn feed(old: &str, new: &str) -> Option<String> {
            let old_c = CString::new(old).unwrap();
            let new_c = CString::new(new).unwrap();
            let raw = am_feed_delta(old_c.as_ptr(), new_c.as_ptr());
            if raw.is_null() {
                return None;
            }
            let out = CStr::from_ptr(raw).to_string_lossy().into_owned();
            am_screen_text_free(raw);
            Some(out)
        }
        unsafe {
            // Mirrors swift TerminalFeedTests case for case over the C ABI.
            assert_eq!(feed("a\nb", "a\nb"), None);
            assert_eq!(feed("", ""), None);
            assert_eq!(feed("hello", "hello world"), Some(" world".to_string()));
            assert_eq!(feed("a", "a\nb\n"), Some("\r\nb\r\n".to_string()));
            assert_eq!(feed("", "ready\n$ "), Some("ready\r\n$ ".to_string()));
            assert_eq!(feed("a\nb", "b\nc"), Some("\r\nc".to_string()));
            assert_eq!(feed("a\nb\nc", "c\nd\ne"), Some("\r\nd\r\ne".to_string()));
            let redraw = feed("menu: [x]", "other screen").expect("redraw replays");
            assert_eq!(redraw, "\x1b[2J\x1b[Hother screen");
            let reflow =
                feed("a very long line here", "a very\nlong line\nhere").expect("reflow replays");
            assert!(reflow.starts_with("\x1b[2J\x1b[H"), "feed: {reflow:?}");
            // Null means empty: the first snapshot still feeds whole.
            let raw = am_feed_delta(std::ptr::null(), CString::new("hi").unwrap().as_ptr());
            assert!(!raw.is_null());
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            am_screen_text_free(raw);
            assert_eq!(text, "hi");
            // Non-UTF8 reports null with a message.
            let bad: [u8; 2] = [0xFF, 0x00];
            assert!(am_feed_delta(
                bad.as_ptr() as *const c_char,
                CString::new("hi").unwrap().as_ptr()
            )
            .is_null());
            let msg = CStr::from_ptr(am_last_error())
                .to_string_lossy()
                .into_owned();
            assert!(msg.contains("am_feed_delta"), "last_error: {msg:?}");
        }
    }

    #[test]
    fn roster_match_age_and_link_count_round_trip() {
        use crate::app::{App, ChatSession, HARNESS_MUSE};
        fn row(id: &str, status: Status, last_active: i64) -> ChatSession {
            ChatSession {
                id: id.into(),
                title: format!("{id} title"),
                project: "proj".into(),
                status,
                harness: HARNESS_MUSE.into(),
                last_active,
                pr_links: vec!["https://github.com/o/r/pull/1".into()],
                related_links: vec!["a".into(), "b".into()],
                links_truncated: false,
                transcript: vec![],
                transcript_truncated: false,
                title_locked: true,
                pending_input: String::new(),
                provider_session_id: None,
                cwd: None,
            }
        }
        let core = AmCore {
            app: App::new(vec![row("a", Status::Attention, 1_700_000_000)]),
        };
        unsafe fn c(s: &str) -> CString {
            CString::new(s).unwrap()
        }
        unsafe {
            let title = c("a title");
            let proj = c("proj");
            let id = c("a");
            let q_title = c("TITLE");
            let q_proj = c("PROJ");
            let q_id = c("A");
            let q_blank = c("  ");
            let q_miss = c("zzz");
            assert_eq!(
                am_roster_matches(title.as_ptr(), proj.as_ptr(), id.as_ptr(), q_blank.as_ptr()),
                1
            );
            assert_eq!(
                am_roster_matches(title.as_ptr(), proj.as_ptr(), id.as_ptr(), q_title.as_ptr()),
                1
            );
            assert_eq!(
                am_roster_matches(title.as_ptr(), proj.as_ptr(), id.as_ptr(), q_proj.as_ptr()),
                1
            );
            assert_eq!(
                am_roster_matches(title.as_ptr(), proj.as_ptr(), id.as_ptr(), q_id.as_ptr()),
                1
            );
            assert_eq!(
                am_roster_matches(title.as_ptr(), proj.as_ptr(), id.as_ptr(), q_miss.as_ptr()),
                0
            );
            // Null query degrades to blank (everything passes).
            assert_eq!(
                am_roster_matches(title.as_ptr(), proj.as_ptr(), id.as_ptr(), std::ptr::null()),
                1
            );
            // Non-UTF8 reports false with a message.
            let bad: [u8; 2] = [0xFF, 0x00];
            assert_eq!(
                am_roster_matches(
                    title.as_ptr(),
                    proj.as_ptr(),
                    id.as_ptr(),
                    bad.as_ptr() as *const c_char
                ),
                0
            );
            let msg = CStr::from_ptr(am_last_error())
                .to_string_lossy()
                .into_owned();
            assert!(msg.contains("am_roster_matches"), "last_error: {msg:?}");

            // Age label: 5 minutes after last_active.
            let raw = am_relative_age(1_700_000_300, 1_700_000_000);
            assert!(!raw.is_null());
            let label = CStr::from_ptr(raw).to_string_lossy().into_owned();
            am_screen_text_free(raw);
            assert_eq!(label, "5m ago");

            // Link count sums PR + related; last_active echoes the row.
            let ptr = &core as *const AmCore;
            assert_eq!(am_link_count(ptr, 0), 3);
            assert_eq!(am_last_active(ptr, 0), 1_700_000_000);
            assert_eq!(am_link_count(ptr, 7), -1);
            assert_eq!(am_last_active(ptr, 7), -1);
            assert_eq!(am_link_count(std::ptr::null(), 0), -1);
        }
    }

    /// Test-only spawn with an explicit argv (the public [`am_spawn`] runs
    /// the configured `muse` session command; tests need fake commands).
    unsafe fn am_spawn_argv(
        core: *const AmCore,
        out: *mut *mut AmPty,
        argv: *const *const c_char,
        argc: usize,
        cwd: *const c_char,
        cols: u16,
        rows: u16,
    ) -> c_int {
        if core.is_null() || out.is_null() {
            set_error("am_spawn: null core or out".to_string());
            return AmError::Null.code();
        }
        if argc > 0 && argv.is_null() {
            set_error("am_spawn: null argv".to_string());
            return AmError::Null.code();
        }
        let mut parts = Vec::with_capacity(argc);
        for i in 0..argc {
            let arg = CStr::from_ptr(*argv.add(i)).to_str();
            match arg {
                Ok(s) => parts.push(s.to_string()),
                Err(_) => {
                    set_error("am_spawn: argv is not valid UTF-8".to_string());
                    return AmError::Utf8.code();
                }
            }
        }
        let (program, args) = match parts.split_first() {
            Some((first, rest)) => (first.clone(), rest.to_vec()),
            None => (*core).app.spawn_command_for(&SpawnKind::New),
        };
        spawn_into(out, &program, &args, cwd, cols, rows)
    }
}
