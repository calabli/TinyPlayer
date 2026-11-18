//! Single-instance support: opening a file while TinyPlayer is already
//! running hands it to the existing window instead of starting another.
//! With the "several windows" setting on, it starts a window of its own
//! instead: another process, independent of the first except for the shared
//! library and settings (see [`is_extra`]).
//!
//! The first instance listens on a loopback TCP port; later instances
//! connect, send their file list, and exit. The port is derived from the
//! data directory, so instances with a different `TINYPLAYER_DATA_DIR`
//! (tests, portable copies) don't talk to each other.

use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

const MAGIC: &str = "TINYPLAYER/1\n";

/// This process is a further window beside one that's already running.
static EXTRA: AtomicBool = AtomicBool::new(false);

/// Whether this is a further window. It leaves what must happen once per
/// session to the first one: the clean-up and the update check at start, the
/// library scan, the window's saved place and the clear-on-exit.
pub fn is_extra() -> bool {
    EXTRA.load(Ordering::Relaxed)
}

/// Added to the names of work folders, so windows don't share one and mix up
/// each other's frames.
pub fn work_tag() -> String {
    if is_extra() { format!("-{}", std::process::id()) } else { String::new() }
}

fn port() -> u16 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    crate::history::data_dir().to_string_lossy().to_lowercase().hash(&mut h);
    // Stay in the dynamic/private range.
    49_200 + (h.finish() % 10_000) as u16
}

fn addr() -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, port())
}

pub enum Role {
    /// We're the running instance. The listener is None if the port was
    /// taken by something else; we then just run without single-instance.
    Primary(Option<TcpListener>),
    /// Another instance took our files; this process should exit.
    Forwarded,
    /// Another instance runs, and this process is a window of its own.
    Extra,
}

/// `several` is the "several windows" setting: files aren't handed to a
/// running instance then.
pub fn acquire(files: &[String], several: bool) -> Role {
    // Try to become the primary instance first: binding is instant, whereas
    // connecting to a port nobody listens on can stall on Windows until the
    // timeout (this used to add ~300 ms to every cold start).
    if let Ok(listener) = TcpListener::bind(addr()) {
        return Role::Primary(Some(listener));
    }
    if several {
        EXTRA.store(true, Ordering::Relaxed);
        return Role::Extra;
    }
    // The port is taken — most likely by a running TinyPlayer.
    if let Ok(mut s) = TcpStream::connect_timeout(&addr().into(), Duration::from_millis(500)) {
        let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
        let mut msg = String::from(MAGIC);
        for f in files {
            // Send absolute paths: the running instance has its own cwd.
            let abs = std::path::absolute(f).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| f.clone());
            msg.push_str(&abs);
            msg.push('\n');
        }
        if s.write_all(msg.as_bytes()).is_ok() {
            // Wait for the ack so we only exit once the files were taken.
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut ack = [0u8; 2];
            if s.read_exact(&mut ack).is_ok() && &ack == b"OK" {
                return Role::Forwarded;
            }
        }
    }
    // Something else holds the port (or the other instance didn't answer):
    // run normally, just without single-instance support.
    Role::Primary(None)
}

/// Starts another TinyPlayer window, with `files` if any. The new process
/// reads the same setting and so becomes a window of its own.
pub fn open_new_window(files: &[String]) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    for f in files {
        cmd.arg(std::path::absolute(f).unwrap_or_else(|_| f.into()));
    }
    let spawned = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        // Collected when it ends, so it doesn't linger as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("new window: {e}"),
    }
}

/// Accepts file lists from later instances on a background thread and sends
/// each list (empty = "just bring the window forward") to `tx`, then calls `wake`.
pub fn serve(listener: TcpListener, tx: Sender<Vec<String>>, wake: impl Fn() + Send + 'static) {
    std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Read until the sender pauses (it doesn't close before our ack).
                loop {
                    match s.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if buf.len() > 1 << 20 {
                                break;
                            }
                            let _ = s.set_read_timeout(Some(Duration::from_millis(150)));
                        }
                        Err(_) => break,
                    }
                }
                let text = String::from_utf8_lossy(&buf);
                let Some(body) = text.strip_prefix(MAGIC) else { continue };
                let files: Vec<String> = body.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
                let _ = s.write_all(b"OK");
                if tx.send(files).is_err() {
                    return;
                }
                wake();
            }
        })
        .ok();
}
