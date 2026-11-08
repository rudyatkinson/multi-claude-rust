//! Starting a program in a pseudo terminal (portable-pty), and the threads that carry its input
//! and output.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::sync::Arc;

use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};

use super::output::{StartupDsr, DSR_ANSWER};
use super::stream::Stream;

/// The size a session starts with, as node-pty was given it.
pub const COLS: u16 = 120;
pub const ROWS: u16 = 30;

/// How much is read from the pty at once.
const READ_CHUNK: usize = 64 * 1024;

/// A program running in a pty, before its threads are started.
pub(crate) struct Opened {
    pub master: Box<dyn MasterPty + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub killer: Box<dyn ChildKiller + Send + Sync>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

/// Starts `program` in a new pty of [`COLS`] x [`ROWS`], in `cwd`, with exactly `env`.
pub(crate) fn open(program: &str, args: &[String], cwd: &str, env: &BTreeMap<String, String>) -> Result<Opened, String> {
    let pair = native_pty_system()
        .openpty(PtySize { rows: ROWS, cols: COLS, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("Could not open a terminal: {e}"))?;
    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    cmd.cwd(cwd);
    cmd.env_clear();
    for (key, value) in env {
        cmd.env(key, value);
    }
    let child = pair.slave.spawn_command(cmd).map_err(|e| format!("Could not start {program}: {e}"))?;
    // the program has the terminal's other end now; holding it here would keep the output open
    drop(pair.slave);
    let killer = child.clone_killer();
    let reader = pair.master.try_clone_reader();
    let writer = pair.master.take_writer();
    match (reader, writer) {
        (Ok(reader), Ok(writer)) => Ok(Opened { master: pair.master, child, killer, reader, writer }),
        (Err(e), _) | (_, Err(e)) => {
            let mut killer = killer;
            let _ = killer.kill();
            Err(format!("Could not connect to the terminal: {e}"))
        }
    }
}

/// The pty's own end as a session keeps it: it is resized, and closed by dropping it, which on
/// Windows closes the pseudoconsole. Tests put one of their own in.
pub(crate) trait Console: Send {
    fn resize(&self, size: PtySize) -> Result<(), String>;
}

impl Console for Box<dyn MasterPty + Send> {
    fn resize(&self, size: PtySize) -> Result<(), String> {
        MasterPty::resize(&**self, size).map_err(|e| e.to_string())
    }
}

pub(crate) fn thread(name: String, f: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    std::thread::Builder::new().name(name).spawn(f).map(|_| ())
}

/// The input of a running session: what is sent here is written to the pty in order, by a
/// thread of its own, so that a program that does not read never blocks the caller.
pub(crate) type Input = mpsc::Sender<Vec<u8>>;

/// The pieces of a running session that stay with the session manager.
pub(crate) struct Started {
    pub master: Box<dyn MasterPty + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub killer: Box<dyn ChildKiller + Send + Sync>,
    pub input: Input,
}

/// Starts the writer, reader and flusher threads of a session. When a thread cannot be started,
/// the program is ended again.
pub(crate) fn start(opened: Opened, stream: Arc<Stream>, tag: &str) -> Result<Started, String> {
    let Opened { master, child, mut killer, reader, mut writer } = opened;
    let (input, queue) = mpsc::channel::<Vec<u8>>();

    let started = thread(format!("pty-write-{tag}"), move || {
        for data in queue {
            if writer.write_all(&data).and_then(|()| writer.flush()).is_err() {
                break;
            }
        }
    })
    .and_then(|()| {
        // Windows: the pseudoconsole waits for the answer to its first request before the program runs
        let answer = cfg!(windows).then(|| input.clone());
        let s = stream.clone();
        thread(format!("pty-read-{tag}"), move || read_loop(reader, &s, answer))
    })
    .and_then(|()| {
        let s = stream.clone();
        thread(format!("pty-flush-{tag}"), move || s.run_flusher())
    });
    if let Err(e) = started {
        let _ = killer.kill();
        stream.close();
        return Err(format!("Could not start the session: {e}"));
    }
    Ok(Started { master, child, killer, input })
}

/// Reads the pty until it has no more output. `answer`: the input to answer the startup cursor
/// request on (see [`StartupDsr`]).
fn read_loop(mut reader: Box<dyn Read + Send>, stream: &Stream, mut answer: Option<Input>) {
    let mut dsr = StartupDsr::new(answer.is_some());
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // EIO on unix once the program and everything it started are gone; a broken pipe on Windows
            Err(_) => break,
        };
        if dsr.is_done() {
            stream.push(&buf[..n]);
            continue;
        }
        let filtered = dsr.filter(&buf[..n]);
        if filtered.answer {
            if let Some(input) = &answer {
                let _ = input.send(DSR_ANSWER.to_vec());
            }
        }
        if dsr.is_done() {
            // the writer thread ends with the session's own input from now on
            answer = None;
        }
        if !filtered.output.is_empty() {
            stream.push(&filtered.output);
        }
    }
    let rest = dsr.finish();
    if !rest.is_empty() {
        stream.push(&rest);
    }
    stream.end();
}
