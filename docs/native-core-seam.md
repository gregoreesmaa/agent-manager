# Native core seam (proposal)

Status: proposal only. No SwiftUI / native shell is built here; this doc
records the audit result, the decoupling already landed, and the exact
core API a future per-OS shell binds against.

Volatility-shield rule (from `VISION-TECHNICAL.md`): no framework types
in the core. Core = everything except `src/gui/` and `src/main.rs`.

## 1. Leak inventory + what moved

Full-repo grep for framework references (`gpui`, `ThemeMode`,
`WindowAppearance`, `Hsla`, `SharedString`, component imports):

| Location | Kind | Verdict |
|---|---|---|
| `src/config.rs` (`ThemePreference::theme_mode`) | **core leak** — signature named `WindowAppearance` and returned `ThemeMode` | **moved** (see below) |
| `src/gui/runs_panel.rs`, `shell.rs`, `terminal_pane.rs`, `nav.rs`, `view.rs` | direct `use gpui` / `use gpui-component` | expected: these are the thin shell |
| `src/gui/terminal.rs` (`to_hsla`) | returns `gpui::Hsla`; rest of the module is plain vt100 math | contained gui-local coupling, left as-is (see §4) |
| `src/main.rs` | `Application`, `Root`, `Theme` | expected: entry point is shell |
| `src/app.rs`, `persist.rs`, `scrollback.rs`, `transcript.rs`, `parsers/`, `providers/`, `embedded.rs` | — | clean, zero framework refs (verified by grep) |

Decoupling landed (minimal, behavior-preserving):

- `config::ThemePreference::theme_mode(Option<WindowAppearance>) -> ThemeMode`
  is replaced by framework-free `resolve(OsAppearance) -> EffectiveTheme`
  plus `is_dark(OsAppearance) -> bool`. New plain types
  `config::OsAppearance::{Dark, Light, Unknown}` (`Unknown` = headless,
  keeps the historic resolve-to-dark default) and
  `config::EffectiveTheme::{Dark, Light}` with `is_dark()`.
- The single framework touchpoint moved to
  `gui::theme::theme_mode_for(ThemePreference, WindowAppearance) -> ThemeMode`,
  used by both `main` (startup) and `shell` (the `t`-key cycle). A framework
  upgrade touches one function; its vibrant-variant mapping is pinned by
  `vibrant_appearances_resolve_to_their_base_mode`, and the core contract
  by `theme_choice_cycles_parses_and_resolves`.
- After the move, `grep -rni gpui` over the core module set returns zero
  hits (code and prose alike), so a future CI gate can enforce it literally.

## 2. Stable core API for native shells

All paths are Rust items inside the current binary crate; §5 notes the
one structural prerequisite (promote core to a library) before any shell
can link them.

### Session roster CRUD (`app`)

- Types: `ChatSession` (serde JSON; stable row key `id`, display-only
  `title`, `project`, `status: Status`, `harness`, `last_active`,
  `pr_links` / `related_links` / `links_truncated`, `transcript` /
  `transcript_truncated`, `title_locked`, `pending_input`,
  `provider_session_id`, `cwd: Option<String>`), `Status::{Attention, Idle,
  Working}`, `Focus`, `App` (`sessions`, `selected`, `filter`,
  `history_expanded` fields).
- Roster: `App::new(sessions)`, `start_new_session()`,
  `start_new_session_in(Option<String>)`, `remove_session(&str)`,
  `selected_session()`, `select_next/select_prev/select_page_next/
  select_page_prev`, `resort_keep_selection()`, `visible_indices()`,
  `matches_filter()`, `new_session_id()`, `sort_sessions()`,
  `status_sections()`, `section_title()`, `attention_count()`.
- Run flow: `take_pending_spawn() -> Option<SpawnKind>`,
  `retry_spawn(SpawnKind)`, `has_pending_spawn()`,
  `respawn_kind(&str) -> SpawnKind`, `session_cwd(&str)`,
  `note_submitted_prompt(&str, &str)`, `spawn_command_for(&SpawnKind) ->
  (String, Vec<String>)`, `spawn_command_string()`.

### Spawn with cwd/flags (`embedded` + `config`)

- `EmbeddedPty::spawn(program, args, cols, rows)` and
  `EmbeddedPty::spawn_with_cwd(program, args, cols, rows,
  Option<&Path>)` — the single spawn seam; `None` cwd inherits the app
  directory. `SpawnKind::{New, Resume { session_id }}` with
  `SpawnKind::command()`.
- Per-agent flags: `Config::extra_args_for(agent) -> Vec<String>`; the
  shell concatenates these onto the spawn command (same as
  `App::spawn_command_for`).

### stdin writes

- `EmbeddedPty::write_input(&[u8]) -> anyhow::Result<()>` — raw bytes,
  already key-encoded by the shell; flush included.

### Output pump / polling (`embedded` + `scrollback`)

- Poll model, no callbacks: the shell runs its own timer (≈20 Hz today)
  and calls `EmbeddedPty::pump() -> bool`; `true` means fresh output or a
  new exit — the only dirty gate the shell needs to repaint.
- Snapshot: `EmbeddedPty::view() -> LiveView { screen: &vt100::Screen,
  exited: bool }`, `resize(cols, rows)`,
  `scrollback_log() -> &ScrollbackLog` (`lines()`, `pending_line()`,
  `line_count()`, `truncated`).
- Portable render helpers a native shell can reuse or port
  (`gui/terminal.rs`, plain functions over vt100 types): `screen_rows`,
  `selection_rows`, `selection_text`, `point_to_cell`,
  `normalize_selection`, `vt_color_for` / `vt_color`, `screen_fingerprint`.
  Only `to_hsla` returns a framework type; its RGB→HSL math is plain and
  ports 1:1 (see §4).

### Status classification (`app`)

- One classifier: `classify(screen_text, output_age, exited) -> Status`,
  `classify_with_attention(bool, output_age, exited)`,
  `needs_attention(&str) -> bool`, `WORKING_WINDOW_SECS` (60),
  `harness_badge(&str)`, `short_cwd(&str)`, `MAX_STORED_LINKS` (50) /
  `MAX_VISIBLE_LINKS` (20), `ChatSession::push_links`,
  `visible_links(&[String])`.

### Persistence (`persist`)

- `load_sessions() / load_sessions_from(&Path)`,
  `save_sessions_to(&Path, &[ChatSession])` (production `save_sessions`
  is `#[cfg(not(test))]` so tests never touch the live file),
  `merge_sessions(discovered, persisted)`, `runs_path()`, `data_dir()`,
  `export_markdown(&ChatSession, &str)`, `write_export()`. Policy the
  shell inherits: missing/corrupt files degrade to empty, never fail
  startup.

### Config get/set (`config` + `App`)

- `Config::load() / load_from(&Path) / save()`, `config_path()`,
  `ThemePreference::{cycle, label, resolve, is_dark}`,
  `TerminalConfig::{font_stack, font_family, font_size, fallback_fonts}`,
  `default_sidebar_width()`; via app: `set_config`, `save_config`,
  `cycle_theme`, `terminal_config`, `sidebar_width`,
  `set_terminal_font_size`, `set_sidebar_width`.

### Link extraction (`parsers`)

- `Parser` trait + `RegistryParser::default()` (fan-out; the seam for new
  content types — no new pipelines per type), `pr_links(&str)`,
  `related_links(&str)`, `github::{extract_pr_links,
  extract_issue_links, extract_commit_links}`,
  `refs::extract_file_refs`, `Parsed { title, project, pr_links,
  related_links }`.

### Historic discovery (`providers` + `transcript`)

- `Provider::discover_sessions() -> Vec<ChatSession>` (never fails;
  unreachable store yields `[]`), `MuseCliProvider::new(store_root,
  Box<dyn Parser>)`, `MuseCliProvider::default_store_root()`.
- `transcript::{parse_transcript, derive_title, single_line,
  extract_project, extract_session_name}`, `TranscriptMessage { role:
  Role, text }`, caps `MAX_TAIL_BYTES` / `MAX_MESSAGES` /
  `MAX_MESSAGE_CHARS` / `MAX_TITLE_CHARS`.

## 3. Binding shape sketches

### Option A — C ABI (`extern "C"` over opaque handles)

```c
// Opaque handles; the shell never sees Rust internals.
typedef struct AmCore AmCore;   // owns App + Config
typedef struct AmPty AmPty;     // owns one EmbeddedPty

AmCore *am_core_new(void);      // discovery + persistence merge inside
void am_core_free(AmCore *);
int32_t am_spawn(AmCore *, AmPty **out, const char *cwd); // 0 = ok
bool am_pump(AmPty *);          // dirty gate
int32_t am_write(AmPty *, const uint8_t *, size_t);
const char *am_screen_text(AmPty *);  // + runs/spans variant for styling
int32_t am_status(AmCore *, size_t row);  // Attention/Idle/Working as int
// Roster/config/persist/link lists cross as JSON (ChatSession serde) or
// small getter batches; errors return as thread-local message strings.
```

Rationale: this seam is narrow and poll-based — spawn, pump, write,
resize, and getters are all plain values in and out, with no async
callbacks, no streaming, and no shared borrowing across the boundary
(`view()` borrows become copied text/spans at the FFI edge). A C ABI
keeps the build trivial (one `staticlib` + modulemap, no codegen phase
in Xcode), gives full control over threading (the shell keeps its own
pump timer, matching today's dirty-gated loop), and forces the one
healthy conversion — borrowed screen snapshots to owned bytes — to
happen explicitly. JSON for roster/config rows reuses the already-tested
serde shapes instead of inventing parallel structs.

### Option B — UniFFI-style generated bindings

```rust
// UDL / proc-macro sketch: records cross by value, PTYs as objects.
record SessionRow { id: String, title: String, project: String,
    status: Status, last_active: i64, pr_links: Vec<String>, … }
enum Status { Attention, Idle, Working }
interface Core {
    constructor(store_root: String);
    Vec<SessionRow> roster();
    u64 spawn(optional String cwd, u16 cols, u16 rows);
}
interface Pty { bool pump(); void write(Vec<u8> bytes);
    string screen_text(); boolean exited(); void resize(u16 c, u16 r); }
```

Rationale: if the surface grows (per-session options, export, provider
configuration) and hand-marshalled JSON/getter batches become their own
bug class, generated Swift (and later Kotlin) bindings remove that class
entirely — records, enums, `Result`, and `Option` map 1:1 and stay in
sync by construction. The price is a new build dependency and Xcode
build phase, core types reshaped into UniFFI-supported shapes, and a
`Send + Sync` audit of everything behind an object handle (the PTY owns
a reader thread and channel today, so interior mutability and the
borrowed `LiveView` need redesigning into owned snapshots first — work
the C ABI also wants, but UniFFI requires up front).

### Recommendation

Start with the C ABI. It matches the seam as it exists — a thin
poll-driven shell over an already framework-free core — with the least
new machinery: no codegen, no dependency, no type reshaping, and the
borrowed-screen-to-owned-text conversion made explicit at one edge.
Revisit UniFFI when a second generated language (Kotlin) is actually on
the roadmap or the getter surface demonstrably outgrows JSON rows; both
paths want the same two prerequisites (§5), so nothing is thrown away.

## 4. Known remaining gui-local coupling (not core leaks)

- `gui/terminal.rs::to_hsla(Rgb8) -> gpui::Hsla`: pure RGB→HSL math
  returning a framework color. A native shell ports the ~25-line math and
  returns its own color (SwiftUI `Color(hue:saturation:brightness:)`).
  If the gpui shell ever needs churn here again, prefer changing the
  return to a plain `{ h, s, l, a: f32 }` struct and converting at the
  three call sites (`terminal_pane.rs`, `view.rs`) — one step closer to a
  shareable terminal-color module.
- `gui/terminal_pane.rs::term_is_light(&gpui::App)` and the font-probe
  helpers take framework contexts; they are render-layer adapters with no
  core equivalent needed (native shells read `EffectiveTheme::is_dark`
  and `TerminalConfig::font_stack` directly).

## 5. Prerequisites before any shell links the core

1. Promote the core to a library: add `[lib] path = "src/core.rs"` (or
   `src/lib.rs`) re-exporting `app`, `config`, `embedded`, `parsers`,
   `persist`, `providers`, `scrollback`, `transcript`, leaving `main.rs`
   + `gui/` as the existing binary consumer. No module moves needed —
   only visibility (`gui`-facing helpers already `pub`/`pub(crate)` where
   the shell needs them).
2. ✅ DONE (slice 2, #60): `EmbeddedPty::snapshot_text()` /
   `snapshot_spans()` / `snapshot()` return owned text + spans
   (`embedded::SnapSpan`-shaped: text + fg/bg/bold/italic/underline as
   plain values; palette duplicated from `gui/terminal.rs`, gui untouched).
   `view()` stays for the gui. No lifetime crosses the FFI boundary.
3. ✅ DONE (slice 2, #60): `src/ffi.rs` maps errors to `AmError` int
   codes (`Ok=0, Spawn=1, Io=2, Utf8=3, Null=4, Config=5`) + a
   thread-local message via `am_last_error()`. Degrade-to-empty on
   missing/corrupt files is preserved (discovery + load inside
   `am_core_new` never fail).

### Landed C ABI (`src/ffi.rs`, crate-type `staticlib` + `rlib`)

Opaque handles (`AmCore` owns `App`+`Config`, `AmPty` owns one
`EmbeddedPty`); plain `#[repr(C)]` `AmRgb` / `AmStyle` (`has_fg`/`has_bg`
presence flags — `Option` stays on the Rust side).

| fn | contract |
|---|---|
| `am_core_new` / `am_core_free` | discovery + persistence merge inside; null-safe free |
| `am_core_save` | persist config; `Config` code on failure |
| `am_spawn(core, out, cwd, cols, rows)` | fresh `muse` session; null cwd inherits; int code |
| `am_pump` | dirty gate; null → false |
| `am_write(pty, bytes, len)` | raw input bytes; int code |
| `am_resize` | null no-op |
| `am_screen_text` + `am_screen_text_free` | owned UTF-8, caller frees |
| `am_spans_json` | owned styled spans as JSON (rows of `{text,fg,bg,bold,italic,underline}`), freed with `am_screen_text_free` |
| `am_status(core, row)` | 0 Attention / 1 Idle / 2 Working; -1 null, -2 out of bounds |
| `am_last_error` | thread-local message; never null |
| `am_pty_free` | reaps the child; null no-op |
