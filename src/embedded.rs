//! Embedded interactive `muse` session behind a PTY with vt100 emulation.
//!
//! The right pane is a live terminal: [`EmbeddedPty`] spawns a child command
//! (normally plain `muse` for a new session) under a PTY from the
//! `portable-pty` crate, feeds every output byte into a [`vt100::Parser`],
//! and forwards focused keystrokes. The parser keeps the full terminal
//! state — cursor addressing, colors, alternate screen — so the UI renders
//! exactly what `muse` draws, instead of a stripped line scrollback.
//! [`LiveView`] is the render snapshot handed to [`crate::ui`].
//!
//! Tests use fake commands (`echo`, shell) so they never need a real `muse`.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use anyhow::{Context, Result};

/// Command used to start a fresh session: plain interactive `muse`.
pub fn new_session_command() -> (String, Vec<String>) {
    ("muse".to_string(), Vec::new())
}

/// What the app asked the PTY layer to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnKind {
    /// Start a brand-new `muse` session.
    New,
    /// Resume a historic provider conversation with `muse --resume <id>`.
    /// The app routes the spawn to the selected run entry, so no run id
    /// travels with the request.
    Resume { session_id: String },
}

impl SpawnKind {
    pub fn command(&self) -> (String, Vec<String>) {
        match self {
            SpawnKind::New => new_session_command(),
            SpawnKind::Resume { session_id } => (
                "muse".to_string(),
                vec!["--resume".to_string(), session_id.clone()],
            ),
        }
    }
}

/// Lines of scrollback retained by the vt100 emulator.
pub const SCROLLBACK_LINES: usize = 2000;

/// A device query is short; keep this many trailing bytes across chunks so a
/// query split over two reads is still recognized.
const QUERY_CARRY: usize = 16;

/// Answer a CPR (cursor-position request) that arrived in `data` (plus
/// `carry` from the previous chunk) with the emulator's current cursor.
/// Returns the reply bytes plus the new carry. A real terminal answers
/// `\x1b[6n` with `\x1b[{row};{col}R` (1-based); without that answer `muse`
/// retries twice and then gives up and exits.
fn cpr_replies(data: &[u8], carry: &[u8], cursor: (u16, u16)) -> (Vec<u8>, Vec<u8>) {
    const QUERY: &[u8] = b"\x1b[6n";
    const DEC_QUERY: &[u8] = b"\x1b[?6n";
    let mut window = Vec::with_capacity(carry.len() + data.len());
    window.extend_from_slice(carry);
    window.extend_from_slice(data);
    let mut replies = Vec::new();
    // Matches wholly inside the old carry were already answered last round —
    // only answer matches that touch new data, otherwise the child would
    // receive duplicate replies as ghost input.
    let old = carry.len();
    let mut i = 0;
    while i + QUERY.len() <= window.len() {
        if window[i..].starts_with(QUERY) {
            if i + QUERY.len() > old {
                replies.extend_from_slice(
                    format!("\x1b[{};{}R", cursor.0 + 1, cursor.1 + 1).as_bytes(),
                );
            }
            i += QUERY.len();
        } else if i + DEC_QUERY.len() <= window.len() && window[i..].starts_with(DEC_QUERY) {
            if i + DEC_QUERY.len() > old {
                replies.extend_from_slice(
                    format!("\x1b[?{};{}R", cursor.0 + 1, cursor.1 + 1).as_bytes(),
                );
            }
            i += DEC_QUERY.len();
        } else {
            i += 1;
        }
    }
    let keep = window.len().min(QUERY_CARRY);
    let new_carry = window[window.len() - keep..].to_vec();
    (replies, new_carry)
}

/// Render snapshot of the live pane, borrowed from the [`EmbeddedPty`]
/// each frame. Issue #32 removed the title row, so the view carries no
/// header text: the screen (with its last frame) is the whole story.
pub struct LiveView<'a> {
    /// Emulated terminal screen (already processed output).
    pub screen: &'a vt100::Screen,
    /// True once the child has exited (last frame stays visible).
    pub exited: bool,
}

/// A live child process behind a PTY with a vt100-emulated screen.
///
/// Output is collected by a background reader thread into a channel; call
/// [`EmbeddedPty::pump`] regularly on the app thread to feed it into the
/// emulator. Input is written straight to the PTY master.
pub struct EmbeddedPty {
    writer: Box<dyn Write + Send>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    rx: Receiver<Vec<u8>>,
    _reader_thread: JoinHandle<()>,
    parser: vt100::Parser,
    query_carry: Vec<u8>,
    exited: bool,
}

impl EmbeddedPty {
    /// Spawn `program` with `args` in a PTY of `cols` x `rows`.
    pub fn spawn(program: &str, args: &[String], cols: u16, rows: u16) -> Result<Self> {
        let spawn_desc = if args.is_empty() {
            program.to_string()
        } else {
            format!("{prog} {args}", prog = program, args = args.join(" "))
        };
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("openpty failed")?;
        let mut cmd = portable_pty::CommandBuilder::new(program);
        cmd.args(args);
        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("failed to spawn `{spawn_desc}`"))?;
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .context("pty take_writer failed")?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("pty try_clone_reader failed")?;
        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();
        let reader_thread = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF: child exited.
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break; // Owner gone.
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            writer,
            _master: pair.master,
            child,
            rx,
            _reader_thread: reader_thread,
            parser: vt100::Parser::new(rows, cols, SCROLLBACK_LINES),
            query_carry: Vec::new(),
            exited: false,
        })
    }

    /// Feed queued output into the emulator. Returns true when new output
    /// arrived or the child newly exited (both change what the UI shows,
    /// so both must dirty the pump). Also polls the child, recording exit
    /// exactly once.
    pub fn pump(&mut self) -> bool {
        let mut fresh = false;
        while let Ok(chunk) = self.rx.try_recv() {
            fresh = true;
            self.ingest(&chunk);
        }
        if !self.exited {
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    fresh = true;
                    self.exited = true;
                    // Feed any last bytes the reader thread already queued.
                    while let Ok(chunk) = self.rx.try_recv() {
                        fresh = true;
                        self.ingest(&chunk);
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    fresh = true;
                    self.exited = true;
                }
            }
        }
        fresh
    }

    /// Forward raw bytes (already key-encoded by the app loop) to the child.
    pub fn write_input(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes).context("pty write failed")?;
        self.writer.flush().context("pty flush failed")?;
        Ok(())
    }

    /// Resize the PTY and the emulator grid; errors after exit are ignored.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self._master.resize(portable_pty::PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.parser.set_size(rows, cols);
    }

    /// Feed one output chunk into the emulator, answering any terminal
    /// queries (currently CPR) the way a real terminal would.
    fn ingest(&mut self, chunk: &[u8]) {
        self.parser.process(chunk);
        let cursor = self.parser.screen().cursor_position();
        let (replies, carry) = cpr_replies(chunk, &self.query_carry, cursor);
        self.query_carry = carry;
        if !replies.is_empty() {
            // Best effort: a closed pipe just means the child is gone.
            let _ = self.writer.write_all(&replies);
            let _ = self.writer.flush();
        }
    }

    /// Borrowed snapshot for the UI.
    pub fn view(&self) -> LiveView<'_> {
        LiveView {
            screen: self.parser.screen(),
            exited: self.exited,
        }
    }

    /// Plain-text contents of the emulated screen (for tests).
    #[cfg(test)]
    fn contents(&mut self) -> String {
        self.pump();
        self.parser.screen().contents()
    }
}

impl Drop for EmbeddedPty {
    fn drop(&mut self) {
        // Best effort: terminate and reap so no zombie survives.
        // try_wait reaps the zombie.
        if !self.exited {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.exited = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn pump_until(pty: &mut EmbeddedPty, needle: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if pty.contents().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        pty.contents().contains(needle)
    }

    fn wait_exit(pty: &mut EmbeddedPty, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            pty.pump();
            if pty.view().exited {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        pty.pump();
        pty.view().exited
    }

    #[test]
    fn cpr_query_is_answered_with_one_based_cursor() {
        let (replies, carry) = cpr_replies(b"abc\x1b[6nxyz", &[], (4, 9));
        assert_eq!(replies, b"\x1b[5;10R");
        assert!(carry.len() <= 16);
    }

    #[test]
    fn cpr_query_split_across_chunks_is_answered_once() {
        let (r1, carry) = cpr_replies(b"ab\x1b[", &[], (0, 0));
        assert!(r1.is_empty());
        let (r2, carry) = cpr_replies(b"6n", &carry, (0, 0));
        assert_eq!(r2, b"\x1b[1;1R");
        // The answered query lingers in the carry but must not re-trigger.
        let (r3, _) = cpr_replies(b"plain", &carry, (0, 0));
        assert!(r3.is_empty());
    }

    #[test]
    fn dec_cpr_variant_is_answered() {
        let (replies, _) = cpr_replies(b"\x1b[?6n", &[], (2, 3));
        assert_eq!(replies, b"\x1b[?3;4R");
    }

    #[test]
    fn new_session_command_shape() {
        assert_eq!(new_session_command(), ("muse".to_string(), vec![]));
        assert_eq!(SpawnKind::New.command(), ("muse".to_string(), vec![]));
    }

    #[test]
    fn resume_command_passes_provider_session_id() {
        // Issue #22: historic re-attach relaunches `muse --resume <id>`.
        let kind = SpawnKind::Resume {
            session_id: "sess-abc".to_string(),
        };
        assert_eq!(
            kind.command(),
            (
                "muse".to_string(),
                vec!["--resume".to_string(), "sess-abc".to_string()]
            )
        );
    }

    #[test]
    fn echo_output_reaches_emulated_screen_without_muse() {
        let mut pty = EmbeddedPty::spawn("echo", &["hello-pty".to_string()], 80, 24)
            .expect("echo must spawn");
        assert!(pump_until(&mut pty, "hello-pty", Duration::from_secs(5)));
        assert!(wait_exit(&mut pty, Duration::from_secs(5)));
    }

    #[test]
    fn cursor_addressing_renders_where_muse_draws() {
        // Direct emulator check: row/col addressing must place text, not
        // append lines — this is what a fullscreen muse TUI relies on.
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[2J\x1b[1;1Htop\x1b[5;10Hmiddle");
        let screen = parser.screen();
        let contents = screen.contents();
        assert!(contents.contains("top"));
        assert!(contents.contains("middle"));
        let cell = screen.cell(4, 9).expect("addressed cell exists");
        assert!(cell.contents().contains("m"));
    }

    #[test]
    fn exit_is_detected_and_view_marks_exited() {
        // Issue #32 removed the header/exit-note text: the exited flag
        // (last frame stays visible) is the whole signal now.
        let mut pty = EmbeddedPty::spawn("true", &[], 80, 24).expect("true must spawn");
        assert!(wait_exit(&mut pty, Duration::from_secs(5)));
        assert!(pty.view().exited);
    }

    #[test]
    fn resize_after_spawn_does_not_panic() {
        let mut pty =
            EmbeddedPty::spawn("sleep", &["5".to_string()], 80, 24).expect("sleep must spawn");
        pty.resize(100, 30);
        pty.resize(80, 24);
    }

    #[test]
    fn input_forwarding_reaches_child() {
        // `head -n 1` echoes its first stdin line then exits: proves bytes
        // written via write_input arrive at the child.
        let mut pty = EmbeddedPty::spawn("head", &["-n".to_string(), "1".to_string()], 80, 24)
            .expect("head must spawn");
        // Give the child a moment to start, then send a line.
        std::thread::sleep(Duration::from_millis(300));
        pty.write_input(b"ping-input\n")
            .expect("write must succeed");
        assert!(pump_until(&mut pty, "ping-input", Duration::from_secs(5)));
    }

    #[test]
    fn spawn_failure_reports_an_error() {
        let err = EmbeddedPty::spawn("definitely-not-a-real-binary-xyz", &[], 80, 24)
            .err()
            .expect("must fail");
        assert!(err.to_string().contains("definitely-not-a-real-binary-xyz"));
    }
}
