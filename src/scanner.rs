//! Background worker: walks watched folders into the library, then probes
//! new files for duration/resolution/thumbnail. Never touches the UI thread
//! except to send short status messages and ask for a repaint.

use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, UNIX_EPOCH};

use eframe::egui;

use crate::library::{self, Library};
use crate::playlist::is_media;
use crate::probe::Prober;

pub enum Cmd {
    ScanAll,
    Quit,
}

#[derive(Debug)]
pub enum Msg {
    /// Library rows changed; reload them.
    Changed,
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
    let thumbs = Library::thumbs_dir();
    // Created on first use; if libmpv can't start, scanning still works
    // (files just have no thumbnail/duration yet).
    let mut prober: Option<Prober> = None;

    while let Ok(cmd) = cmds.recv() {
        if matches!(cmd, Cmd::Quit) {
            return;
        }
        let mut again = true;
        while again {
            again = false;
            walk_all(&lib);
            send(Msg::Changed);

            let pending = lib.pending_probe();
            if pending.is_empty() {
                continue;
            }
            if prober.is_none() {
                prober = Prober::new(&crate::history::data_dir()).ok();
            }
            let Some(pr) = &prober else { continue };
            let total = pending.len();
            let mut last_flush = Instant::now();
            for (i, (id, path)) in pending.into_iter().enumerate() {
                match cmds.try_recv() {
                    Ok(Cmd::Quit) | Err(TryRecvError::Disconnected) => return,
                    Ok(Cmd::ScanAll) => again = true,
                    Err(TryRecvError::Empty) => {}
                }
                let dest = thumbs.join(format!("{id}.jpg"));
                let info = pr.probe(Path::new(&path), &dest);
                lib.set_probe_result(id, info.as_ref());
                send(Msg::Probing(i + 1, total));
                if last_flush.elapsed() > Duration::from_millis(700) {
                    send(Msg::Changed);
                    last_flush = Instant::now();
                }
            }
            send(Msg::Changed);
        }
        send(Msg::Idle);
    }
}

fn walk_all(lib: &Library) {
    for folder in lib.folders() {
        let mut seen = HashSet::new();
        walk(Path::new(&folder.path), 0, &mut |path, meta| {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            lib.upsert_file(folder.id, path, meta.len(), mtime);
            seen.insert(library::display_path(path));
        });
        lib.mark_missing(folder.id, &seen);
    }
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
