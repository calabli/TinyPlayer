//! Background worker: walks watched folders into the library, then probes
//! new files for duration/resolution/thumbnail. Never touches the UI thread
//! except to send short status messages and ask for a repaint.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant, UNIX_EPOCH};

use eframe::egui;

use crate::library::{self, Library};
use crate::playlist::is_media;
use crate::probe::{Info, Prober};

/// More would mostly fight over the disk.
const MAX_PROBERS: usize = 3;
const REPORT_EVERY: Duration = Duration::from_millis(250);
const COMMIT_EVERY: Duration = Duration::from_secs(1);
const RELOAD_EVERY: Duration = Duration::from_secs(5);
const WRITE_BATCH: usize = 500;

pub enum Cmd {
    ScanAll,
    Quit,
}

#[derive(Debug)]
pub enum Msg {
    /// Library rows changed; reload them.
    Changed,
    /// Walking the folders: files seen so far.
    Scanning(usize),
    /// Thumbnail progress: (done, total).
    Probing(usize, usize),
    Idle,
}

pub struct Scanner {
    tx: Sender<Cmd>,
    pub rx: Receiver<Msg>,
}

impl Scanner {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, cmd_rx) = mpsc::channel();
        let (msg_tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("library-scanner".into())
            .spawn(move || worker(cmd_rx, msg_tx, ctx))
            .expect("spawn scanner");
        Self { tx, rx }
    }

    pub fn scan(&self) {
        let _ = self.tx.send(Cmd::ScanAll);
    }

    pub fn quit(&self) {
        let _ = self.tx.send(Cmd::Quit);
    }
}

fn worker(cmds: Receiver<Cmd>, out: Sender<Msg>, ctx: egui::Context) {
    let send = |m: Msg| {
        let _ = out.send(m);
        ctx.request_repaint();
    };
    let Ok(lib) = Library::open(&Library::db_path()) else { return };

    while let Ok(cmd) = cmds.recv() {
        if matches!(cmd, Cmd::Quit) {
            return;
        }
        let mut again = true;
        while again {
            again = false;
            walk_all(&lib, &send);
            send(Msg::Changed);
            let pending = lib.pending_probe();
            if !pending.is_empty() && probe_all(&lib, pending, &cmds, &send, &mut again) {
                return;
            }
            send(Msg::Changed);
        }
        send(Msg::Idle);
    }
}

/// Probes files on a few threads, each with its own mpv instance, which is
/// released when done (tens of MB each). If libmpv can't start, scanning still
/// works; files just have no thumbnail or duration yet. Results are written
/// here, on one connection. Returns true if the app is quitting.
fn probe_all(lib: &Library, pending: Vec<(i64, String)>, cmds: &Receiver<Cmd>, send: &dyn Fn(Msg), again: &mut bool) -> bool {
    let total = pending.len();
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get() / 2).clamp(1, MAX_PROBERS);
    let thumbs = Library::thumbs_dir();
    let queue = Mutex::new(VecDeque::from(pending));
    let (tx, results) = mpsc::channel();
    let data = crate::history::data_dir();
    let mut quit = false;

    std::thread::scope(|scope| {
        for n in 0..workers {
            let (tx, queue, thumbs, work_dir) = (tx.clone(), &queue, &thumbs, data.join(format!("probe-{n}")));
            scope.spawn(move || {
                let Ok(prober) = Prober::new(&work_dir) else { return };
                loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some((id, path)) = next else { break };
                    let info = prober.probe(Path::new(&path), &thumbs.join(format!("{id}.jpg")));
                    if tx.send((id, info)).is_err() {
                        break;
                    }
                }
                drop(prober);
                let _ = std::fs::remove_dir_all(&work_dir);
            });
        }
        drop(tx);

        let mut done = 0;
        let mut unsaved = Vec::new();
        let (mut last_commit, mut last_report, mut last_reload) = (Instant::now(), Instant::now(), Instant::now());
        loop {
            match results.recv_timeout(Duration::from_millis(200)) {
                Ok(result) => {
                    unsaved.push(result);
                    done += 1;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            match cmds.try_recv() {
                Ok(Cmd::Quit) | Err(TryRecvError::Disconnected) => {
                    quit = true;
                    if let Ok(mut q) = queue.lock() {
                        q.clear();
                    }
                }
                Ok(Cmd::ScanAll) => *again = true,
                Err(TryRecvError::Empty) => {}
            }
            if last_report.elapsed() >= REPORT_EVERY {
                send(Msg::Probing(done, total));
                last_report = Instant::now();
            }
            if last_commit.elapsed() >= COMMIT_EVERY {
                save_probes(lib, &mut unsaved);
                last_commit = Instant::now();
                // Reloading a big library isn't free, so it's kept occasional.
                if last_reload.elapsed() >= RELOAD_EVERY {
                    send(Msg::Changed);
                    last_reload = Instant::now();
                }
            }
        }
        save_probes(lib, &mut unsaved);
        send(Msg::Probing(done, total));
    });
    quit
}

/// Writes in one short burst: holding the database between files would keep
/// the UI's own writes waiting.
fn save_probes(lib: &Library, results: &mut Vec<(i64, Option<Info>)>) {
    lib.begin();
    for (id, info) in results.drain(..) {
        lib.set_probe_result(id, info.as_ref());
    }
    lib.commit();
}

fn walk_all(lib: &Library, send: &dyn Fn(Msg)) {
    let mut count = 0;
    let mut last_report = Instant::now();
    for folder in lib.folders() {
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        walk(Path::new(&folder.path), 0, &mut |path, meta| {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            seen.insert(library::display_path(path));
            found.push((path.to_path_buf(), meta.len(), mtime));
            count += 1;
            if found.len() >= WRITE_BATCH {
                save_files(lib, folder.id, &mut found);
            }
            if last_report.elapsed() >= REPORT_EVERY {
                send(Msg::Scanning(count));
                last_report = Instant::now();
            }
        });
        save_files(lib, folder.id, &mut found);
        lib.mark_missing(folder.id, &seen);
    }
    send(Msg::Scanning(count));
}

fn save_files(lib: &Library, folder: i64, found: &mut Vec<(PathBuf, u64, i64)>) {
    lib.begin();
    for (path, size, mtime) in found.drain(..) {
        lib.upsert_file(folder, &path, size, mtime);
    }
    lib.commit();
}

const MAX_DEPTH: usize = 12;

fn walk(dir: &Path, depth: usize, f: &mut dyn FnMut(&Path, &std::fs::Metadata)) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.starts_with('$') || name == "System Volume Information" {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            walk(&path, depth + 1, f);
        } else if meta.is_file() && is_media(&path) {
            f(&path, &meta);
        }
    }
}
