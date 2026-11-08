//! A session's output on its way to the window: the reader thread pushes what the pty prints,
//! the flusher thread sends it 8 ms after it started waiting (or at once past 64K units), and
//! `attach` sends what waits before it takes the replay. One lock orders all of it, so `seq`
//! only grows in the order the events go out.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use mc_core::types::PtyData;

use super::output::{OutputBuffer, FLUSH_MS, FLUSH_UNITS};
use super::{now_ms, Outlet};

#[derive(Default)]
struct State {
    buf: OutputBuffer,
    /// When the oldest output that waits to be sent arrived.
    first_pending: Option<Instant>,
    /// The pty has no more output.
    eof: bool,
    /// The tab was closed: nothing more is sent.
    closed: bool,
}

pub(crate) struct Stream {
    id: String,
    state: Mutex<State>,
    wake: Condvar,
    outlet: Arc<dyn Outlet>,
    /// When the pty last printed something (ms since the epoch); when the stream was made before that.
    last_output: AtomicI64,
}

impl Stream {
    pub(crate) fn new(id: String, outlet: Arc<dyn Outlet>) -> Arc<Self> {
        Self::with_buffer(id, outlet, OutputBuffer::default())
    }

    /// The stream of a process that takes over from the one of `before` in the same tab: its
    /// replay and `seq` go on from there (see [`OutputBuffer::carried_over`]).
    pub(crate) fn following(id: String, outlet: Arc<dyn Outlet>, before: &Stream) -> Arc<Self> {
        let buf = before.lock().buf.carried_over();
        Self::with_buffer(id, outlet, buf)
    }

    fn with_buffer(id: String, outlet: Arc<dyn Outlet>, buf: OutputBuffer) -> Arc<Self> {
        let state = State { buf, ..State::default() };
        Arc::new(Stream { id, state: Mutex::new(state), wake: Condvar::new(), outlet, last_output: AtomicI64::new(now_ms()) })
    }

    /// When the pty last printed something; when the stream was made, before that.
    pub(crate) fn last_output_at(&self) -> i64 {
        self.last_output.load(Ordering::Relaxed)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // a panic elsewhere must not take the terminal down with it
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sends what waits, under the lock, so that chunks go out in the order of their `seq`.
    fn flush_locked(&self, st: &mut State) {
        st.first_pending = None;
        if st.closed {
            st.buf.discard_pending();
            return;
        }
        if let Some((data, seq)) = st.buf.take() {
            self.outlet.data(PtyData { id: self.id.clone(), data, seq });
        }
    }

    /// Output read from the pty.
    pub(crate) fn push(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.last_output.store(now_ms(), Ordering::Relaxed);
        }
        let mut st = self.lock();
        if st.closed {
            // a process that outlives its tab must not fill memory
            return;
        }
        if st.buf.push(bytes) >= FLUSH_UNITS {
            self.flush_locked(&mut st);
        } else if st.first_pending.is_none() && st.buf.has_pending() {
            st.first_pending = Some(Instant::now());
            self.wake.notify_all();
        }
    }

    /// The pty has no more output.
    pub(crate) fn end(&self) {
        let mut st = self.lock();
        st.buf.finish();
        if st.buf.has_pending() && st.first_pending.is_none() {
            st.first_pending = Some(Instant::now());
        }
        st.eof = true;
        self.wake.notify_all();
    }

    /// Sends what waits now.
    pub(crate) fn flush(&self) {
        let mut st = self.lock();
        self.flush_locked(&mut st);
    }

    /// Sends what waits, then the replay and the sequence number after it.
    pub(crate) fn attach(&self) -> (String, u64) {
        let mut st = self.lock();
        self.flush_locked(&mut st);
        (st.buf.replay().to_owned(), st.buf.seq())
    }

    /// The tab is gone: what waits is dropped and nothing more is sent.
    pub(crate) fn close(&self) {
        let mut st = self.lock();
        st.closed = true;
        st.buf.discard_pending();
        st.first_pending = None;
        self.wake.notify_all();
    }

    /// Waits until the pty has no more output, at most `timeout`. True when it ended.
    pub(crate) fn wait_end(&self, timeout: Duration) -> bool {
        let st = self.lock();
        let (st, _) = self.wake.wait_timeout_while(st, timeout, |st| !st.eof).unwrap_or_else(|e| e.into_inner());
        st.eof
    }

    /// The flusher thread: sends output [`FLUSH_MS`] after the first of it arrived, until the
    /// pty has no more output and nothing waits.
    pub(crate) fn run_flusher(&self) {
        let delay = Duration::from_millis(FLUSH_MS);
        let mut st = self.lock();
        loop {
            match st.first_pending {
                None if st.eof || st.closed => return,
                None => st = self.wake.wait(st).unwrap_or_else(|e| e.into_inner()),
                Some(first) => {
                    let due = first + delay;
                    let now = Instant::now();
                    if st.eof || st.closed || now >= due {
                        self.flush_locked(&mut st);
                    } else {
                        st = self.wake.wait_timeout(st, due - now).unwrap_or_else(|e| e.into_inner()).0;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_core::types::SessionsChanged;

    #[derive(Default)]
    struct Recorder {
        data: Mutex<Vec<PtyData>>,
    }

    impl Outlet for Recorder {
        fn data(&self, payload: PtyData) {
            self.data.lock().unwrap().push(payload);
        }
        fn sessions(&self, _payload: SessionsChanged) {}
    }

    impl Recorder {
        fn sent(&self) -> Vec<(String, u64)> {
            self.data.lock().unwrap().iter().map(|d| (d.data.clone(), d.seq)).collect()
        }
    }

    fn stream() -> (Arc<Recorder>, Arc<Stream>) {
        let recorder = Arc::new(Recorder::default());
        let stream = Stream::new("s1".into(), recorder.clone());
        (recorder, stream)
    }

    fn with_flusher(stream: &Arc<Stream>) -> std::thread::JoinHandle<()> {
        let s = stream.clone();
        std::thread::spawn(move || s.run_flusher())
    }

    #[test]
    fn output_waits_a_moment_and_goes_out_together() {
        let (rec, stream) = stream();
        let flusher = with_flusher(&stream);
        stream.push(b"a");
        stream.push(b"b");
        // nothing yet: the chunk waits for what follows
        assert!(rec.sent().is_empty());
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(rec.sent(), vec![("ab".to_owned(), 2)]);
        stream.push("😀".as_bytes());
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(rec.sent(), vec![("ab".to_owned(), 2), ("😀".to_owned(), 4)]);
        stream.end();
        flusher.join().unwrap();
    }

    #[test]
    fn a_long_chunk_goes_out_at_once() {
        let (rec, stream) = stream();
        // no flusher: only the size can send it
        let big = "z".repeat(FLUSH_UNITS);
        stream.push(big.as_bytes());
        assert_eq!(rec.sent(), vec![(big, FLUSH_UNITS as u64)]);
    }

    #[test]
    fn attach_sends_what_waits_and_returns_the_replay() {
        let (rec, stream) = stream();
        stream.push(b"hello ");
        stream.flush();
        stream.push(b"world");
        let (replay, seq) = stream.attach();
        assert_eq!(replay, "hello world");
        assert_eq!(seq, 11);
        assert_eq!(rec.sent(), vec![("hello ".to_owned(), 6), ("world".to_owned(), 11)]);
    }

    #[test]
    fn the_end_sends_the_rest_and_stops_the_flusher() {
        let (rec, stream) = stream();
        let flusher = with_flusher(&stream);
        stream.push(b"last\xe2");
        stream.end();
        flusher.join().unwrap();
        assert_eq!(rec.sent(), vec![("last\u{fffd}".to_owned(), 5)]);
        assert!(stream.wait_end(Duration::from_millis(1)));
    }

    #[test]
    fn a_closed_stream_sends_nothing_more() {
        let (rec, stream) = stream();
        let flusher = with_flusher(&stream);
        stream.push(b"never");
        stream.close();
        flusher.join().unwrap();
        stream.push(b"again");
        stream.flush();
        assert!(rec.sent().is_empty());
    }

    #[test]
    fn a_following_stream_goes_on_with_the_seq_and_notes_when_output_came() {
        let (rec, before) = stream();
        before.push(b"old ");
        before.end();
        before.flush();
        let next = Stream::following("s1".into(), rec.clone(), &before);
        assert_eq!(next.attach(), ("old ".to_owned(), 4));
        let started = next.last_output_at();
        std::thread::sleep(Duration::from_millis(20));
        next.push(b"new");
        next.flush();
        assert!(next.last_output_at() > started);
        assert_eq!(rec.sent(), vec![("old ".to_owned(), 4), ("new".to_owned(), 7)]);
        assert_eq!(next.attach(), ("old new".to_owned(), 7));
    }

    #[test]
    fn waiting_for_the_end_times_out() {
        let (_rec, stream) = stream();
        let started = Instant::now();
        assert!(!stream.wait_end(Duration::from_millis(30)));
        assert!(started.elapsed() >= Duration::from_millis(30));
    }
}
