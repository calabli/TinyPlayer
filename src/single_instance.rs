//! Single-instance support: opening a file while TinyPlayer is already
//! running hands it to the existing window instead of starting another.
//!
//! The first instance listens on a loopback TCP port; later instances
//! connect, send their file list, and exit. The port is derived from the
//! data directory, so instances with a different `TINYPLAYER_DATA_DIR`
//! (tests, portable copies) don't talk to each other.

use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::time::Duration;

const MAGIC: &str = "TINYPLAYER/1\n";

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
}

pub fn acquire(files: &[String]) -> Role {
    // Try to become the primary instance first: binding is instant, whereas
    // connecting to a port nobody listens on can stall on Windows until the
    // timeout (this used to add ~300 ms to every cold start).
    if let Ok(listener) = TcpListener::bind(addr()) {
        return Role::Primary(Some(listener));
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
