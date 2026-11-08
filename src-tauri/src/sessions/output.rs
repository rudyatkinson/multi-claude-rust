//! The pure part of a session's output: decoding the pty's bytes, the chunk waiting to be sent,
//! the replay buffer and the sequence number (`flush` and `onData` of `src/main/sessions.ts`).
//!
//! Every length here is a JavaScript `length` (UTF-16 units), as it was in the Electron version:
//! the replay keeps the last [`REPLAY_LIMIT`] units, a chunk is sent once it reaches
//! [`FLUSH_UNITS`], and `seq` counts the units sent so far.

use mc_core::jsstr::len16;

/// How much output a session keeps for a window that attaches later, in UTF-16 units.
pub const REPLAY_LIMIT: usize = 256 * 1024;
/// How long output may wait to be sent with what follows it.
pub const FLUSH_MS: u64 = 8;
/// A chunk this long is sent at once.
pub const FLUSH_UNITS: usize = 64 * 1024;

/// Decodes a byte stream as UTF-8 the way Node's `StringDecoder` does: an invalid sequence is a
/// U+FFFD, and a character whose bytes are split between two reads is decoded once it is whole.
#[derive(Debug, Default)]
pub struct Utf8Carry {
    carry: Vec<u8>,
}

impl Utf8Carry {
    /// Appends the text of `bytes` to `out`, keeping an incomplete character at the end for the next call.
    pub fn decode(&mut self, bytes: &[u8], out: &mut String) {
        if self.carry.is_empty() {
            let rest = decode_into(bytes, out);
            self.carry.extend_from_slice(rest);
        } else {
            let mut joined = std::mem::take(&mut self.carry);
            joined.extend_from_slice(bytes);
            let rest = decode_into(&joined, out);
            self.carry = rest.to_vec();
        }
    }

    /// The end of the stream: an incomplete character left over is one U+FFFD (`decoder.end()`).
    pub fn finish(&mut self, out: &mut String) {
        if !self.carry.is_empty() {
            self.carry.clear();
            out.push(char::REPLACEMENT_CHARACTER);
        }
    }
}

/// Decodes what can be decoded and returns the incomplete character at the end, if any.
fn decode_into<'a>(mut input: &'a [u8], out: &mut String) -> &'a [u8] {
    loop {
        match std::str::from_utf8(input) {
            Ok(text) => {
                out.push_str(text);
                return &[];
            }
            Err(e) => {
                let (valid, after) = input.split_at(e.valid_up_to());
                // valid_up_to says this part is UTF-8
                out.push_str(std::str::from_utf8(valid).unwrap_or_default());
                match e.error_len() {
                    Some(bad) => {
                        out.push(char::REPLACEMENT_CHARACTER);
                        input = &after[bad..];
                    }
                    // the input ends inside a character
                    None => return after,
                }
            }
        }
    }
}

/// The byte index from which `s` (of `units` UTF-16 units) keeps at most `max` units, and how
/// many units lie before it. Never inside a character: where `slice(-max)` would split a
/// surrogate pair, the whole character is left out.
fn tail_start(s: &str, units: usize, max: usize) -> (usize, usize) {
    if units <= max {
        return (0, 0);
    }
    let mut dropped = 0usize;
    for (idx, ch) in s.char_indices() {
        if units - dropped <= max {
            return (idx, dropped);
        }
        dropped += ch.len_utf16();
    }
    (s.len(), dropped)
}

/// A session's output: the chunk not sent yet, what has been sent (for the replay) and how much.
#[derive(Debug, Default)]
pub struct OutputBuffer {
    decoder: Utf8Carry,
    pending: String,
    pending_units: usize,
    /// The sent output. It may hold up to twice [`REPLAY_LIMIT`], so that it is cut only now
    /// and then; [`OutputBuffer::replay`] returns the last [`REPLAY_LIMIT`] units of it.
    replay: String,
    replay_units: usize,
    seq: u64,
}

impl OutputBuffer {
    /// Adds bytes read from the pty; returns the units now waiting to be sent.
    pub fn push(&mut self, bytes: &[u8]) -> usize {
        let before = self.pending.len();
        self.decoder.decode(bytes, &mut self.pending);
        self.pending_units += len16(&self.pending[before..]);
        self.pending_units
    }

    /// The pty has no more output: a character it left incomplete becomes U+FFFD.
    pub fn finish(&mut self) {
        let before = self.pending.len();
        self.decoder.finish(&mut self.pending);
        self.pending_units += len16(&self.pending[before..]);
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    #[cfg(test)]
    pub fn pending_units(&self) -> usize {
        self.pending_units
    }

    /// `flush`: the chunk to send and the sequence number after it, or None when nothing waits.
    /// The chunk joins the replay.
    pub fn take(&mut self) -> Option<(String, u64)> {
        if self.pending.is_empty() {
            return None;
        }
        let data = std::mem::take(&mut self.pending);
        let units = std::mem::take(&mut self.pending_units);
        self.seq += units as u64;
        self.replay.push_str(&data);
        self.replay_units += units;
        if self.replay_units > 2 * REPLAY_LIMIT {
            let (at, dropped) = tail_start(&self.replay, self.replay_units, REPLAY_LIMIT);
            self.replay.drain(..at);
            self.replay_units -= dropped;
        }
        Some((data, self.seq))
    }

    /// Drops what waits to be sent (a closed tab sends nothing more).
    pub fn discard_pending(&mut self) {
        self.pending.clear();
        self.pending_units = 0;
    }

    /// The last [`REPLAY_LIMIT`] units of the output sent so far.
    pub fn replay(&self) -> &str {
        let (at, _) = tail_start(&self.replay, self.replay_units, REPLAY_LIMIT);
        &self.replay[at..]
    }

    /// UTF-16 units of output sent so far.
    pub fn seq(&self) -> u64 {
        self.seq
    }
}

/// Windows only: the cursor position request (`ESC [ 6 n`) a pseudoconsole sends before anything
/// else. portable-pty creates it with `PSEUDOCONSOLE_INHERIT_CURSOR`, and the console then waits
/// for the answer before the program runs at all. node-pty did not ask for that, so the session
/// answers it at once (the cursor of a fresh terminal is at 1;1) and the window never sees the
/// request: an answer from the window, which may attach much later or again after a reload,
/// would reach the program as typed text.
///
/// Newer consoles switch modes first (`ESC [ ? 9001 h` for the win32 input mode portable-pty asks
/// for, `ESC [ ? 1004 h` for focus reports), so private mode sequences in front of the request
/// are passed on and looked past, for at most [`DSR_SCAN_LIMIT`] bytes.
///
/// Not asking for the request at all would be better, but portable-pty 0.9 passes its creation
/// flags to `CreatePseudoConsole` as constants (`win/psuedocon.rs`): no feature, environment
/// variable or `PtySystem` option changes them, short of a fork of the crate. Hence this filter.
#[derive(Debug)]
pub struct StartupDsr {
    seen: Vec<u8>,
    done: bool,
}

/// The request a pseudoconsole starts with.
pub const DSR_REQUEST: &[u8] = b"\x1b[6n";
/// The answer: row 1, column 1.
pub const DSR_ANSWER: &[u8] = b"\x1b[1;1R";
/// How far into the output the request is looked for, past the mode sequences in front of it.
pub const DSR_SCAN_LIMIT: usize = 64;

/// What the start of the output is, as far as the startup request goes.
#[derive(Debug, PartialEq, Eq)]
enum Scan {
    /// The request begins at this byte, after mode sequences only.
    Found(usize),
    /// Not enough output yet to tell.
    Wait,
    /// Something else came first: there is no request to answer.
    Absent,
}

/// How a private mode sequence (`ESC [ ? <digits> h` or `... l`) at the start of some output stands.
enum Mode {
    /// One of this many bytes.
    Complete(usize),
    /// Could still become one.
    Incomplete,
    No,
}

fn private_mode(bytes: &[u8]) -> Mode {
    const START: &[u8] = b"\x1b[?";
    if bytes.len() < START.len() {
        return if START.starts_with(bytes) { Mode::Incomplete } else { Mode::No };
    }
    if !bytes.starts_with(START) {
        return Mode::No;
    }
    let digits = bytes[START.len()..].iter().take_while(|b| b.is_ascii_digit() || **b == b';').count();
    match bytes.get(START.len() + digits) {
        None => Mode::Incomplete,
        Some(b'h' | b'l') if digits > 0 => Mode::Complete(START.len() + digits + 1),
        Some(_) => Mode::No,
    }
}

/// Looks past the mode sequences at the start of `seen` for the request.
fn scan(seen: &[u8]) -> Scan {
    let mut at = 0;
    loop {
        if at >= DSR_SCAN_LIMIT {
            return Scan::Absent;
        }
        let rest = &seen[at..];
        if rest.starts_with(DSR_REQUEST) {
            return Scan::Found(at);
        }
        let waiting = if rest.len() < DSR_REQUEST.len() && DSR_REQUEST.starts_with(rest) {
            true
        } else {
            match private_mode(rest) {
                Mode::Complete(len) => {
                    at += len;
                    continue;
                }
                Mode::Incomplete => true,
                Mode::No => false,
            }
        };
        return if waiting && seen.len() < DSR_SCAN_LIMIT { Scan::Wait } else { Scan::Absent };
    }
}

/// What [`StartupDsr::filter`] made of a read.
#[derive(Debug, PartialEq, Eq)]
pub struct Filtered {
    /// The output to pass on (possibly empty).
    pub output: Vec<u8>,
    /// The request was found: answer it now.
    pub answer: bool,
}

impl StartupDsr {
    /// `active` false passes everything through (other systems, or no request expected).
    pub fn new(active: bool) -> Self {
        StartupDsr { seen: Vec::new(), done: !active }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Looks for the request at the very start of the output (past mode sequences), and only there.
    pub fn filter(&mut self, bytes: &[u8]) -> Filtered {
        if self.done {
            return Filtered { output: bytes.to_vec(), answer: false };
        }
        self.seen.extend_from_slice(bytes);
        let found = match scan(&self.seen) {
            // could still be the request: wait for the rest of it
            Scan::Wait => return Filtered { output: Vec::new(), answer: false },
            Scan::Found(at) => Some(at),
            Scan::Absent => None,
        };
        self.done = true;
        let mut seen = std::mem::take(&mut self.seen);
        match found {
            Some(at) => {
                // the mode sequences in front of it go on to the window
                seen.drain(at..at + DSR_REQUEST.len());
                Filtered { output: seen, answer: true }
            }
            None => Filtered { output: seen, answer: false },
        }
    }

    /// The end of the output: whatever was held back goes on.
    pub fn finish(&mut self) -> Vec<u8> {
        self.done = true;
        std::mem::take(&mut self.seen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(parts: &[&[u8]]) -> String {
        let mut carry = Utf8Carry::default();
        let mut out = String::new();
        for part in parts {
            carry.decode(part, &mut out);
        }
        carry.finish(&mut out);
        out
    }

    #[test]
    fn decodes_characters_split_between_reads() {
        let text = "aı😀b";
        let bytes = text.as_bytes();
        // every way of cutting it in two gives the same text
        for cut in 0..=bytes.len() {
            assert_eq!(decode_all(&[&bytes[..cut], &bytes[cut..]]), text, "cut at {cut}");
        }
        // one byte at a time
        let singles: Vec<&[u8]> = bytes.chunks(1).collect();
        assert_eq!(decode_all(&singles), text);
    }

    #[test]
    fn invalid_bytes_become_replacement_characters() {
        assert_eq!(decode_all(&[b"a\xffb"]), "a\u{fffd}b");
        assert_eq!(decode_all(&[b"\xe2\x28\xa1"]), "\u{fffd}(\u{fffd}");
        // a character the stream never completes
        assert_eq!(decode_all(&[b"ok\xf0\x9f"]), "ok\u{fffd}");
    }

    #[test]
    fn nothing_waits_while_a_character_is_incomplete() {
        let mut out = OutputBuffer::default();
        assert_eq!(out.push(&"😀".as_bytes()[..2]), 0);
        assert!(!out.has_pending());
        assert_eq!(out.take(), None);
        assert_eq!(out.push(&"😀".as_bytes()[2..]), 2);
        assert_eq!(out.take(), Some(("😀".to_owned(), 2)));
    }

    #[test]
    fn seq_counts_utf16_units() {
        let mut out = OutputBuffer::default();
        out.push("abc".as_bytes());
        assert_eq!(out.take(), Some(("abc".to_owned(), 3)));
        out.push("Işık😀".as_bytes());
        assert_eq!(out.pending_units(), 6);
        assert_eq!(out.take(), Some(("Işık😀".to_owned(), 9)));
        assert_eq!(out.seq(), 9);
        assert_eq!(out.take(), None);
        assert_eq!(out.seq(), 9);
    }

    #[test]
    fn chunks_gather_until_taken() {
        let mut out = OutputBuffer::default();
        out.push(b"one ");
        out.push(b"two");
        assert_eq!(out.pending_units(), 7);
        assert_eq!(out.take().unwrap().0, "one two");
        assert_eq!(out.pending_units(), 0);
        assert_eq!(out.replay(), "one two");
    }

    #[test]
    fn replay_keeps_the_last_256k_units() {
        let mut out = OutputBuffer::default();
        let block = "x".repeat(FLUSH_UNITS);
        for i in 0..10 {
            out.push(block.as_bytes());
            out.push(format!("{i}").as_bytes());
            out.take();
            assert!(len16(out.replay()) <= REPLAY_LIMIT);
        }
        let replay = out.replay();
        assert_eq!(len16(replay), REPLAY_LIMIT);
        assert!(replay.ends_with('9'));
        // what is kept is the end of everything sent
        assert_eq!(out.seq(), 10 * (FLUSH_UNITS as u64 + 1));
        // the store itself never grows past twice the limit
        assert!(out.replay_units <= 2 * REPLAY_LIMIT);
    }

    #[test]
    fn replay_is_cut_at_a_character() {
        let mut out = OutputBuffer::default();
        // a surrogate pair right where the cut falls: one unit too many
        out.push(format!("😀{}", "y".repeat(REPLAY_LIMIT - 1)).as_bytes());
        out.take();
        let replay = out.replay();
        assert!(!replay.starts_with('😀'));
        assert_eq!(len16(replay), REPLAY_LIMIT - 1);
        // exactly at the limit nothing is cut
        let mut exact = OutputBuffer::default();
        exact.push(format!("😀{}", "y".repeat(REPLAY_LIMIT - 2)).as_bytes());
        exact.take();
        assert!(exact.replay().starts_with('😀'));
    }

    #[test]
    fn discarded_output_is_never_sent() {
        let mut out = OutputBuffer::default();
        out.push(b"gone");
        out.discard_pending();
        assert_eq!(out.take(), None);
        assert_eq!(out.seq(), 0);
        assert_eq!(out.replay(), "");
    }

    #[test]
    fn finish_flushes_an_incomplete_character() {
        let mut out = OutputBuffer::default();
        out.push(b"a\xe2\x82");
        out.finish();
        assert_eq!(out.take(), Some(("a\u{fffd}".to_owned(), 2)));
    }

    #[test]
    fn the_startup_request_is_answered_and_removed() {
        let mut dsr = StartupDsr::new(true);
        assert_eq!(dsr.filter(b"\x1b[6n\x1b[?9001h"), Filtered { output: b"\x1b[?9001h".to_vec(), answer: true });
        assert!(dsr.is_done());
        // later requests belong to the window
        assert_eq!(dsr.filter(b"\x1b[6n"), Filtered { output: b"\x1b[6n".to_vec(), answer: false });
    }

    #[test]
    fn the_startup_request_may_come_in_pieces() {
        let mut dsr = StartupDsr::new(true);
        assert_eq!(dsr.filter(b"\x1b"), Filtered { output: vec![], answer: false });
        assert_eq!(dsr.filter(b"[6"), Filtered { output: vec![], answer: false });
        assert_eq!(dsr.filter(b"nhi"), Filtered { output: b"hi".to_vec(), answer: true });
    }

    #[test]
    fn other_output_first_means_no_startup_request() {
        let mut dsr = StartupDsr::new(true);
        assert_eq!(dsr.filter(b"\x1b["), Filtered { output: vec![], answer: false });
        assert_eq!(dsr.filter(b"31mred \x1b[6n"), Filtered { output: b"\x1b[31mred \x1b[6n".to_vec(), answer: false });
        let mut plain = StartupDsr::new(true);
        assert_eq!(plain.filter(b"hello"), Filtered { output: b"hello".to_vec(), answer: false });
        // an incomplete start is handed on at the end
        let mut cut = StartupDsr::new(true);
        cut.filter(b"\x1b[");
        assert_eq!(cut.finish(), b"\x1b[".to_vec());
        // inactive: everything passes
        let mut off = StartupDsr::new(false);
        assert_eq!(off.filter(b"\x1b[6n"), Filtered { output: b"\x1b[6n".to_vec(), answer: false });
    }

    #[test]
    fn mode_sequences_before_the_startup_request_are_passed_on() {
        let start = b"\x1b[?1004h\x1b[?9001h\x1b[6n\x1b[c";
        let mut dsr = StartupDsr::new(true);
        assert_eq!(dsr.filter(start), Filtered { output: b"\x1b[?1004h\x1b[?9001h\x1b[c".to_vec(), answer: true });
        assert!(dsr.is_done());
        // the same one byte at a time: nothing goes on until the request is found
        let mut pieces = StartupDsr::new(true);
        let mut output = Vec::new();
        let mut answers = 0;
        for byte in start.chunks(1) {
            let filtered = pieces.filter(byte);
            output.extend(filtered.output);
            answers += usize::from(filtered.answer);
        }
        assert_eq!((output, answers), (b"\x1b[?1004h\x1b[?9001h\x1b[c".to_vec(), 1));
        // a cursor hidden first, and a lot of output with the request
        let mut long = StartupDsr::new(true);
        let mut read = b"\x1b[?25l\x1b[6n".to_vec();
        read.extend(std::iter::repeat_n(b'x', 10_000));
        let filtered = long.filter(&read);
        assert!(filtered.answer);
        assert_eq!(filtered.output.len(), 6 + 10_000);
    }

    #[test]
    fn the_request_is_looked_for_only_so_far() {
        // mode sequences longer than the limit: a request after them belongs to the window
        let modes = b"\x1b[?1004h".repeat(DSR_SCAN_LIMIT / 8 + 1);
        let mut dsr = StartupDsr::new(true);
        let read = [modes.as_slice(), DSR_REQUEST].concat();
        assert_eq!(dsr.filter(&read), Filtered { output: read.clone(), answer: false });
        // endless digits are not waited for
        let mut digits = StartupDsr::new(true);
        let mut endless = b"\x1b[?".to_vec();
        endless.extend(std::iter::repeat_n(b'1', DSR_SCAN_LIMIT));
        assert_eq!(digits.filter(&endless), Filtered { output: endless.clone(), answer: false });
        // anything else after a mode sequence ends the search
        let mut other = StartupDsr::new(true);
        assert_eq!(other.filter(b"\x1b[?9001hhi\x1b[6n"), Filtered { output: b"\x1b[?9001hhi\x1b[6n".to_vec(), answer: false });
        let mut no_digits = StartupDsr::new(true);
        assert_eq!(no_digits.filter(b"\x1b[?h\x1b[6n"), Filtered { output: b"\x1b[?h\x1b[6n".to_vec(), answer: false });
        // a mode sequence that never ends is handed on at the end
        let mut cut = StartupDsr::new(true);
        assert_eq!(cut.filter(b"\x1b[?90"), Filtered { output: vec![], answer: false });
        assert_eq!(cut.finish(), b"\x1b[?90".to_vec());
    }
}
