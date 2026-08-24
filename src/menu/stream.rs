//! Streaming stdin: incremental line parsing and redraw coalescing.
//!
//! The menu no longer blocks on `read_to_end` before it can open: items are
//! appended as they arrive while the menu is already on screen. Two pieces
//! make that work:
//!
//! - [`LineParser`] turns arbitrary byte chunks into item lines with the
//!   exact semantics of the old blocking read (one trailing `\n` or `\t`
//!   stripped per line, cut at the first NUL, invalid UTF-8 dropped).
//! - [`Gate`] decides when accumulated arrivals are worth a rematch + redraw
//!   ("settle"): after 1 ms of quiet, or 16 ms after the first arrival of a
//!   window, whichever comes first. A fast producer that writes everything
//!   within one window therefore produces exactly one settle — the same
//!   single rematch + redraw the blocking version did — while a slow
//!   trickle still updates the menu at least every [`Gate::MAX_AGE`].

use std::time::{Duration, Instant};

/// Settle once stdin has been quiet for this long. Small enough to be
/// invisible (a frame at 60 Hz is 16.7 ms), large enough that a producer
/// doing many small writes collapses into one settle.
pub(super) const QUIET: Duration = Duration::from_millis(1);

/// Upper bound for how long a settle may be deferred by continuing
/// arrivals. Without it, a producer emitting a line just under every
/// [`QUIET`] would keep the window open forever and the menu would never
/// update until EOF.
pub(super) const MAX_AGE: Duration = Duration::from_millis(16);

/// Incremental port of the blocking stdin parse. Feed raw bytes as they are
/// read; complete lines come out as items. [`LineParser::finish`] flushes
/// the trailing partial line (a final chunk without a newline is still an
/// item); [`LineParser::spurious_lone_newline`] reports the one case where
/// the incremental parse disagrees with the blocking read (see below).
#[derive(Debug, Default)]
pub(super) struct LineParser {
    buf: Vec<u8>,
    /// Total bytes ever fed.
    total_len: usize,
    /// First byte of the stream, for the lone-newline quirk.
    first_byte: Option<u8>,
}

impl LineParser {
    /// Parse complete lines out of `chunk`.
    pub(super) fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, &b) in chunk.iter().enumerate() {
            if b == b'\n' {
                self.buf.extend_from_slice(&chunk[start..i]);
                if let Some(line) = parse_line(&self.buf) {
                    lines.push(line);
                }
                self.buf.clear();
                start = i + 1;
            }
        }
        self.buf.extend_from_slice(&chunk[start..]);
        self.total_len += chunk.len();
        if self.first_byte.is_none() {
            self.first_byte = chunk.first().copied();
        }
        lines
    }

    /// Flush at end-of-stream. A trailing chunk without a final newline is
    /// still an item.
    pub(super) fn finish(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.buf.is_empty() {
            if let Some(line) = parse_line(&self.buf) {
                lines.push(line);
            }
            self.buf.clear();
        }
        lines
    }

    /// C parity quirk: `printf '\n' | instantmenu` produced zero items,
    /// because the blocking read stripped the one trailing newline off the
    /// whole input and split an empty remainder. The incremental parse has
    /// already emitted that empty item by the time EOF is known, so the
    /// caller drops it. Every other input segments identically.
    pub(super) fn spurious_lone_newline(&self) -> bool {
        self.total_len == 1 && self.first_byte == Some(b'\n')
    }
}

/// One line: strip ONE trailing `\t`, cut at the first NUL, drop invalid
/// UTF-8 (the C strdup kept the bytes, but items are drawn as text — the
/// blocking port dropped such lines and did not count them either).
fn parse_line(raw: &[u8]) -> Option<String> {
    let raw = raw.strip_suffix(b"\t").unwrap_or(raw);
    let cut = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    std::str::from_utf8(&raw[..cut]).ok().map(str::to_string)
}

/// Redraw-coalescing window for streamed items. Arrivals open/extend the
/// window; the menu settles when it closes.
#[derive(Debug, Default)]
pub(super) struct Gate {
    opened: Option<Instant>,
    last_arrival: Option<Instant>,
}

impl Gate {
    /// Record an arrival at `now`, opening the window on the first one.
    pub(super) fn note_arrival(&mut self, now: Instant) {
        self.opened.get_or_insert(now);
        self.last_arrival = Some(now);
    }

    /// Time left until the window must close, regardless of further
    /// arrivals. `None` when no arrival has opened a window yet (the poll
    /// may block indefinitely; the stdin fd wakes it).
    pub(super) fn budget(&self, now: Instant) -> Option<Duration> {
        let quiet_deadline = self.last_arrival?.checked_add(QUIET)?;
        let age_deadline = self.opened?.checked_add(MAX_AGE)?;
        Some(
            quiet_deadline
                .min(age_deadline)
                .saturating_duration_since(now),
        )
    }

    /// The window has run out: time to settle.
    pub(super) fn expired(&self, now: Instant) -> bool {
        self.budget(now).is_some_and(|d| d.is_zero())
    }

    /// Close the current window; the next arrival opens a fresh one.
    pub(super) fn reset(&mut self) {
        self.opened = None;
    }
}

/* ── streaming shell glue ─────────────────────────────────────────────── */

use std::io::ErrorKind;

use super::matcher::Item;
use super::Menu;
use crate::enums::ExitStatus;

/// Keep startup work bounded even when stdin is continuously writable. One
/// pipe-sized chunk is enough to avoid the empty-layout flash for ordinary
/// producers; anything beyond it is consumed by the event loop.
const PRELOAD_MAX_BYTES: usize = 64 * 1024;

impl Menu {
    /// Opportunistic preload before the first layout: if the producer has
    /// already written (e.g. `echo ... | instantmenu`), consume what is
    /// buffered without blocking so `setup()` measures the final width
    /// instead of the empty `auto` fallback (which previously flashed
    /// full-monitor width before shrinking). Does not block; a slow
    /// producer still streams normally after the window is visible.
    /// Resolves fallback fonts for the preloaded characters so the first
    /// draw measures correctly. For `auto` this eliminates the wide flash
    /// entirely for fast producers (instantstartmenu); for slow producers
    /// the geometry fallback (content_width) bounds the initial flash.
    pub fn preload_available(&mut self) {
        if self.stream_fd < 0 {
            return;
        }
        self.drain_stdin_up_to(PRELOAD_MAX_BYTES);
        if !self.pending_chars.is_empty() {
            let chars = std::mem::take(&mut self.pending_chars);
            self.renderer.add_fallbacks(&chars);
            // Those items are now part of the baseline corpus for setup(),
            // not a pending dirty batch that needs a later settle/reflow.
            self.gate.reset();
            self.stream_dirty = false;
        }
    }

    /// Read everything currently available from the streaming stdin
    /// (O_NONBLOCK: read until it would block), parse complete lines and
    /// append them as items. Returns true when EOF was reached. Bytes are
    /// never held back: a producer blocked on a full pipe is unblocked by
    /// this drain, and the coalescing gate decides when the batch becomes
    /// visible.
    pub(crate) fn drain_stdin(&mut self) -> bool {
        self.drain_stdin_up_to(usize::MAX)
    }

    /// Drain at most `max_bytes`, leaving a readable descriptor for the event
    /// loop when the budget is exhausted. The cap is essential before setup:
    /// a producer such as `yes` may otherwise prevent window creation forever.
    fn drain_stdin_up_to(&mut self, max_bytes: usize) -> bool {
        let fd = self.stream_fd;
        let mut eof = false;
        let mut bytes_read = 0usize;
        loop {
            let mut buf = [0u8; 64 * 1024];
            let remaining = max_bytes.saturating_sub(bytes_read);
            if remaining == 0 {
                break;
            }
            let read_len = buf.len().min(remaining);
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), read_len) };
            match n {
                0 => {
                    eof = true;
                    break;
                }
                n if n > 0 => {
                    bytes_read += n as usize;
                    let lines = self.parser.feed(&buf[..n as usize]);
                    if !lines.is_empty() {
                        /* only completed items open the coalescing window:
                         * a partial line sitting in the buffer changes
                         * nothing visible yet */
                        self.gate.note_arrival(std::time::Instant::now());
                        self.add_items(lines.into_iter().map(Item::new).collect());
                    }
                }
                _ => {
                    match std::io::Error::last_os_error().kind() {
                        ErrorKind::Interrupted => continue,
                        /* EAGAIN/EWOULDBLOCK: drained for now; anything else
                         * is treated the same rather than killing the menu
                         * over a dead pipe */
                        _ => break,
                    }
                }
            }
        }
        if eof && !self.stream_eof {
            self.stream_eof = true;
            let tail = self.parser.finish();
            if !tail.is_empty() {
                self.add_items(tail.into_iter().map(Item::new).collect());
            }
            if self.parser.spurious_lone_newline() {
                self.matcher.items.pop();
            }
            self.stream_dirty = true;
        }
        eof
    }

    /// A settle is due: EOF has not been finalized yet, or items arrived
    /// and the coalescing window closed.
    pub(super) fn stream_settle_due(&self, now: std::time::Instant) -> bool {
        (self.stream_fd >= 0 && self.stream_eof && !self.stream_finalized)
            || (self.stream_dirty && self.gate.expired(now))
    }

    /// The poll timeout for the coalescing window; None lets the poll block
    /// until a backend event or stdin data arrives.
    pub(super) fn stream_poll_budget(
        &self,
        now: std::time::Instant,
    ) -> Option<std::time::Duration> {
        if !self.stream_active() {
            return None;
        }
        self.gate.budget(now)
    }

    /// A batch settled: resolve fonts for the new characters in one
    /// fontconfig pass, install the layout derived from the grown corpus,
    /// re-match/paginate against that layout, and redraw once. Pick
    /// conclusions stay deferred while the corpus is incomplete.
    pub(super) fn settle_stream(&mut self) {
        let chars = std::mem::take(&mut self.pending_chars);
        self.renderer.add_fallbacks(&chars);
        self.reflow();
        let _ = self.do_match();
        self.draw_menu();
        self.gate.reset();
        self.stream_dirty = false;
    }

    /// EOF settled: the corpus is final, so the deferred conclusions fire
    /// in the order the blocking startup used to run them — match (which
    /// may auto-confirm and exit), then pre-match, then the deferred
    /// preselection — followed by the final layout pass and draw.
    pub(super) fn finalize_stream(&mut self) -> Option<ExitStatus> {
        self.stream_finalized = true;
        let chars = std::mem::take(&mut self.pending_chars);
        self.renderer.add_fallbacks(&chars);
        /* Layout precedes every consumer derived from it. In particular,
         * do_match calculates page boundaries and deferred preselection may
         * cross them; neither may run against the initial empty layout. */
        self.reflow();
        let t = self.do_match();
        if let Some(status) = self.perform(t) {
            return Some(status);
        }
        if let Some(status) = self.apply_pre_match() {
            return Some(status);
        }
        self.apply_deferred_preselect();
        self.draw_menu();
        self.stream_dirty = false;
        None
    }

    /// --preselect at EOF, when the first-event application ran before any
    /// items existed.
    fn apply_deferred_preselect(&mut self) {
        for _ in 0..self.cfg.preselected.max(0) {
            self.select_next();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /* ── LineParser ──────────────────────────────────────────────────── */

    #[test]
    fn parser_splits_on_newlines() {
        let mut p = LineParser::default();
        assert_eq!(p.feed(b"alpha\nbe"), vec!["alpha".to_string()]);
        assert_eq!(p.feed(b"ta\n"), vec!["beta".to_string()]);
        assert_eq!(p.finish(), Vec::<String>::new());
    }

    /// A final chunk without a trailing newline is still an item.
    #[test]
    fn parser_final_chunk_without_newline_is_an_item() {
        let mut p = LineParser::default();
        assert_eq!(p.feed(b"a\nb"), vec!["a".to_string()]);
        assert_eq!(p.finish(), vec!["b".to_string()]);
    }

    /// Empty input yields no items.
    #[test]
    fn parser_empty_input_yields_no_items() {
        let mut p = LineParser::default();
        assert!(p.feed(b"").is_empty());
        assert!(p.finish().is_empty());
    }

    /// C parity: an input of exactly one newline yields no items (the
    /// blocking read stripped the trailing newline and had nothing left).
    /// The incremental parse emitted the empty item, so the caller pops it.
    #[test]
    fn parser_lone_newline_yields_no_items() {
        let mut p = LineParser::default();
        assert_eq!(p.feed(b"\n"), vec!["".to_string()]);
        assert!(p.finish().is_empty());
        assert!(p.spurious_lone_newline());
    }

    /// Any other single-byte input keeps its item.
    #[test]
    fn parser_other_single_byte_inputs_keep_their_item() {
        let mut p = LineParser::default();
        assert!(p.feed(b"a").is_empty());
        assert_eq!(p.finish(), vec!["a".to_string()]);
        assert!(!p.spurious_lone_newline());

        let mut p = LineParser::default();
        assert!(p.feed(b"\t").is_empty());
        assert_eq!(p.finish(), vec!["".to_string()]); // tab stripped
        assert!(!p.spurious_lone_newline());
    }

    /// ...but two newlines yield two empty items, like the blocking read.
    #[test]
    fn parser_two_newlines_yield_two_empty_items() {
        let mut p = LineParser::default();
        assert_eq!(p.feed(b"\n\n"), vec!["".to_string(), "".to_string()]);
        assert!(p.finish().is_empty());
    }

    /// Empty lines between items are kept.
    #[test]
    fn parser_keeps_empty_lines() {
        let mut p = LineParser::default();
        assert_eq!(
            p.feed(b"a\n\nb\n"),
            vec!["a".to_string(), "".to_string(), "b".to_string()]
        );
        assert!(p.finish().is_empty());
    }

    /// ONE trailing tab per line is stripped (the comment separator).
    #[test]
    fn parser_strips_one_trailing_tab() {
        let mut p = LineParser::default();
        assert_eq!(
            p.feed(b"name\tcomment\t\nx\t\n"),
            vec!["name\tcomment".to_string(), "x".to_string()]
        );
    }

    /// Lines are cut at the first NUL like strdup would.
    #[test]
    fn parser_cuts_at_nul() {
        let mut p = LineParser::default();
        assert_eq!(
            p.feed(b"ab\0cd\nef\n"),
            vec!["ab".to_string(), "ef".to_string()]
        );
    }

    /// Invalid UTF-8 drops the line entirely — it is not counted, matching
    /// the blocking read's drop-lossy behaviour.
    #[test]
    fn parser_drops_invalid_utf8_lines() {
        let mut p = LineParser::default();
        let mut chunk = b"ok\n".to_vec();
        chunk.extend_from_slice(&[0xff, 0xfe, b'\n']);
        chunk.extend_from_slice(b"fine\n");
        assert_eq!(p.feed(&chunk), vec!["ok".to_string(), "fine".to_string()]);
    }

    /// Multi-byte characters split across chunks reassemble correctly
    /// (newlines cannot appear inside a UTF-8 sequence).
    #[test]
    fn parser_reassembles_split_multibyte_chars() {
        let text = "hätive";
        let bytes = text.as_bytes();
        let mid = 2; // inside the ä

        // no newline yet: everything buffers until one arrives
        let mut p = LineParser::default();
        assert!(p.feed(&bytes[..mid]).is_empty());
        assert!(p.feed(&bytes[mid..]).is_empty());
        assert_eq!(p.finish(), vec![text.to_string()]);

        // a newline after the split character flushes the reassembled line
        let mut p = LineParser::default();
        assert!(p.feed(&bytes[..mid]).is_empty());
        let mut rest = bytes[mid..].to_vec();
        rest.extend_from_slice(b"\nx\n");
        assert_eq!(p.feed(&rest), vec![text.to_string(), "x".to_string()]);
    }

    /* ── Gate ────────────────────────────────────────────────────────── */

    fn at(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    /// No arrival, no window: the poll may block indefinitely.
    #[test]
    fn gate_without_arrivals_never_expires() {
        let g = Gate::default();
        assert_eq!(g.budget(at(0)), None);
        assert!(!g.expired(at(1000)));
    }

    /// A single arrival closes the window after QUIET.
    #[test]
    fn gate_closes_after_quiet() {
        let t0 = at(0);
        let mut g = Gate::default();
        g.note_arrival(t0);
        assert!(g.budget(t0).unwrap() > Duration::from_millis(0));
        assert!(!g.expired(t0));
        assert!(g.expired(t0 + QUIET));
    }

    /// Continued arrivals extend the quiet deadline...
    #[test]
    fn gate_arrivals_extend_the_quiet_deadline() {
        let t0 = at(0);
        let mut g = Gate::default();
        g.note_arrival(t0);
        // still inside the window shortly before the original deadline
        g.note_arrival(t0 + QUIET - Duration::from_micros(200));
        assert!(!g.expired(t0 + QUIET));
    }

    /// ...but never past MAX_AGE from the first arrival.
    #[test]
    fn gate_max_age_bounds_the_window() {
        let t0 = at(0);
        let mut g = Gate::default();
        g.note_arrival(t0);
        // a steady trickle just under QUIET can extend forever otherwise
        for i in 1..40 {
            g.note_arrival(t0 + QUIET * i - Duration::from_micros(100));
        }
        assert!(g.expired(t0 + MAX_AGE));
        assert!(!g.expired(t0 + MAX_AGE - Duration::from_millis(1)));
    }

    /// After a reset the next arrival opens a fresh window.
    #[test]
    fn gate_reset_opens_a_fresh_window() {
        let t0 = at(0);
        let mut g = Gate::default();
        g.note_arrival(t0);
        g.reset();
        assert_eq!(g.budget(t0 + QUIET), None);
        let t1 = t0 + Duration::from_millis(50);
        g.note_arrival(t1);
        assert!(!g.expired(t1));
        assert!(g.expired(t1 + QUIET));
    }
}
