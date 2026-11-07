//! Moves video files on a background thread. Within a disk that's a rename.
//! Across disks the file is copied under a temporary name and read back; the
//! original goes only after the copy is known to be good.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use sha2::{Digest, Sha256};

const CHUNK: usize = 1 << 20;
const PART_SUFFIX: &str = ".tinyplayer-part";

#[derive(Clone, Debug)]
pub struct Job {
    pub id: i64,
    pub from: PathBuf,
    pub to_dir: PathBuf,
    /// The library folder `to_dir` is in.
    pub folder_id: i64,
}

/// A whole folder, with everything in it.
#[derive(Clone, Debug)]
pub struct FolderJob {
    pub from: PathBuf,
    pub to_dir: PathBuf,
    pub folder_id: i64,
}

#[derive(Clone, Debug)]
pub enum Task {
    File(Job),
    Folder(FolderJob),
}

#[derive(Debug)]
pub enum Msg {
    Started { name: String },
    Copied { done: u64, total: u64 },
    Verifying,
    Moved { job: Job, to: PathBuf },
    FolderMoved { job: FolderJob, to: PathBuf },
    Failed,
    Finished,
}

/// `name` in `dir`, or the first free `name (2)`, `name (3)`…
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match Path::new(name).extension() {
        Some(e) => (Path::new(name).file_stem().map_or(name.into(), |s| s.to_string_lossy()), format!(".{}", e.to_string_lossy())),
        None => (name.into(), String::new()),
    };
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Whether moving between these is a rename rather than a copy.
pub fn same_disk(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        matches!((fs::metadata(a), fs::metadata(b)), (Ok(x), Ok(y)) if x.dev() == y.dev())
    }
    #[cfg(not(unix))]
    {
        a.components().next() == b.components().next()
    }
}

fn hash_file(path: &Path, cancel: &AtomicBool) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hasher.finalize().to_vec());
        }
        hasher.update(&buf[..n]);
    }
}

fn copy_then_verify(from: &Path, part: &Path, cancel: &AtomicBool, tx: &Sender<Msg>) -> io::Result<()> {
    let mut src = File::open(from)?;
    let meta = src.metadata()?;
    let mut dst = OpenOptions::new().write(true).create_new(true).open(part)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        dst.write_all(&buf[..n])?;
        done += n as u64;
        let _ = tx.send(Msg::Copied { done, total: meta.len() });
    }
    dst.sync_all()?;
    if let Ok(modified) = meta.modified() {
        let _ = dst.set_modified(modified);
    }
    drop(dst);
    let _ = tx.send(Msg::Verifying);
    if hash_file(part, cancel)? != hasher.finalize().to_vec() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(())
}

/// Moves one file. Whatever fails, the original is still in place.
fn move_file(from: &Path, to_dir: &Path, cancel: &AtomicBool, tx: &Sender<Msg>) -> io::Result<PathBuf> {
    let name = from.file_name().ok_or(io::ErrorKind::InvalidInput)?.to_string_lossy().into_owned();
    let to = free_path(to_dir, &name);
    match fs::rename(from, &to) {
        Ok(()) => return Ok(to),
        Err(e) if e.kind() != io::ErrorKind::CrossesDevices => return Err(e),
        Err(_) => {}
    }
    let part = to_dir.join(format!("{name}{PART_SUFFIX}"));
    let copied = copy_then_verify(from, &part, cancel, tx).and_then(|()| {
        let to = free_path(to_dir, &name);
        fs::rename(&part, &to).map(|()| to)
    });
    match copied {
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
        Ok(to) => {
            if let Err(e) = fs::remove_file(from) {
                // Locked, say. Two copies would be worse than a failed move.
                let _ = fs::remove_file(&to);
                return Err(e);
            }
            Ok(to)
        }
    }
}

fn copy_tree(from: &Path, to: &Path, cancel: &AtomicBool, tx: &Sender<Msg>) -> io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target, cancel, tx)?;
        } else if kind.is_file() {
            let _ = tx.send(Msg::Started { name: entry.file_name().to_string_lossy().into_owned() });
            copy_then_verify(&entry.path(), &target, cancel, tx)?;
        } else {
            // A link's target could be anywhere, so it can't be moved safely.
            return Err(io::ErrorKind::Unsupported.into());
        }
    }
    Ok(())
}

/// Moves a folder. Whatever fails, the original is still complete.
fn move_folder(from: &Path, to_dir: &Path, cancel: &AtomicBool, tx: &Sender<Msg>) -> io::Result<PathBuf> {
    let name = from.file_name().ok_or(io::ErrorKind::InvalidInput)?.to_string_lossy().into_owned();
    let to = free_path(to_dir, &name);
    match fs::rename(from, &to) {
        Ok(()) => return Ok(to),
        Err(e) if e.kind() != io::ErrorKind::CrossesDevices => return Err(e),
        Err(_) => {}
    }
    let part = to_dir.join(format!("{name}{PART_SUFFIX}"));
    let copied = copy_tree(from, &part, cancel, tx).and_then(|()| {
        let to = free_path(to_dir, &name);
        fs::rename(&part, &to).map(|()| to)
    });
    match copied {
        Err(e) => {
            let _ = fs::remove_dir_all(&part);
            Err(e)
        }
        Ok(to) => {
            // The copy is complete and checked, so it stays even if some
            // originals can't be cleared.
            let _ = fs::remove_dir_all(from);
            Ok(to)
        }
    }
}

pub struct Mover {
    pub rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Mover {
    pub fn start(tasks: Vec<Task>, wake: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("file-move".into())
            .spawn(move || {
                for task in tasks {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let from = match &task {
                        Task::File(job) => &job.from,
                        Task::Folder(job) => &job.from,
                    };
                    let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let _ = tx.send(Msg::Started { name });
                    wake();
                    let done = match task {
                        Task::File(job) => move_file(&job.from, &job.to_dir, &stop, &tx).map(|to| Msg::Moved { job, to }),
                        Task::Folder(job) => move_folder(&job.from, &job.to_dir, &stop, &tx).map(|to| Msg::FolderMoved { job, to }),
                    };
                    match done {
                        Ok(msg) => {
                            let _ = tx.send(msg);
                        }
                        Err(_) if stop.load(Ordering::Relaxed) => {}
                        Err(_) => {
                            let _ = tx.send(Msg::Failed);
                        }
                    }
                    wake();
                }
                let _ = tx.send(Msg::Finished);
                wake();
            })
            .ok();
        Self { rx, cancel, worker }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Quitting mid-copy shouldn't leave a partial file behind.
impl Drop for Mover {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tinyplayer-move-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn free_names_keep_the_extension() {
        let dir = temp("names");
        assert_eq!(free_path(&dir, "a.mp4"), dir.join("a.mp4"));
        fs::write(dir.join("a.mp4"), b"x").unwrap();
        assert_eq!(free_path(&dir, "a.mp4"), dir.join("a (2).mp4"));
        fs::write(dir.join("a (2).mp4"), b"x").unwrap();
        assert_eq!(free_path(&dir, "a.mp4"), dir.join("a (3).mp4"));
        fs::write(dir.join("b"), b"x").unwrap();
        assert_eq!(free_path(&dir, "b"), dir.join("b (2)"));
    }

    #[test]
    fn a_copy_is_verified_and_keeps_the_date() {
        let dir = temp("copy");
        let (from, part) = (dir.join("a.mp4"), dir.join("a.part"));
        fs::write(&from, vec![7u8; CHUNK + 5]).unwrap();
        let (tx, rx) = mpsc::channel();
        copy_then_verify(&from, &part, &AtomicBool::new(false), &tx).unwrap();
        assert_eq!(fs::read(&part).unwrap().len(), CHUNK + 5);
        assert!(rx.try_iter().any(|m| matches!(m, Msg::Verifying)));
        let time = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(time(&from), time(&part));
    }

    #[test]
    fn cancelling_a_copy_keeps_the_original() {
        let dir = temp("cancel");
        let (from, to_dir) = (dir.join("a.mp4"), dir.join("t"));
        fs::create_dir(&to_dir).unwrap();
        fs::write(&from, b"data").unwrap();
        let part = to_dir.join("a.part");
        let (tx, _rx) = mpsc::channel();
        assert!(copy_then_verify(&from, &part, &AtomicBool::new(true), &tx).is_err());
        assert_eq!(fs::read(&from).unwrap(), b"data");
    }

    #[test]
    fn a_folder_tree_copies_with_everything_in_it() {
        let dir = temp("tree");
        let from = dir.join("trip");
        fs::create_dir_all(from.join("day 1")).unwrap();
        fs::write(from.join("a.mp4"), b"video").unwrap();
        fs::write(from.join("a.srt"), b"subs").unwrap();
        fs::write(from.join("day 1/b.mp4"), b"more").unwrap();
        let to = dir.join("copy");
        let (tx, _rx) = mpsc::channel();
        copy_tree(&from, &to, &AtomicBool::new(false), &tx).unwrap();
        assert_eq!(fs::read(to.join("a.srt")).unwrap(), b"subs");
        assert_eq!(fs::read(to.join("day 1/b.mp4")).unwrap(), b"more");
    }

    #[test]
    fn a_folder_moves_on_one_disk() {
        let dir = temp("folder");
        let from = dir.join("trip");
        fs::create_dir_all(&from).unwrap();
        fs::write(from.join("a.mp4"), b"video").unwrap();
        fs::create_dir(dir.join("t")).unwrap();
        fs::create_dir(dir.join("t/trip")).unwrap();
        let (tx, _rx) = mpsc::channel();
        let to = move_folder(&from, &dir.join("t"), &AtomicBool::new(false), &tx).unwrap();
        assert_eq!(to, dir.join("t/trip (2)"));
        assert!(to.join("a.mp4").exists());
        assert!(!from.exists());
    }

    #[test]
    fn a_rename_moves_and_avoids_a_clash() {
        let dir = temp("rename");
        let (from, to_dir) = (dir.join("a.mp4"), dir.join("t"));
        fs::create_dir(&to_dir).unwrap();
        fs::write(&from, b"new").unwrap();
        fs::write(to_dir.join("a.mp4"), b"old").unwrap();
        let (tx, _rx) = mpsc::channel();
        let to = move_file(&from, &to_dir, &AtomicBool::new(false), &tx).unwrap();
        assert_eq!(to, to_dir.join("a (2).mp4"));
        assert_eq!(fs::read(to_dir.join("a.mp4")).unwrap(), b"old");
        assert_eq!(fs::read(to).unwrap(), b"new");
        assert!(!from.exists());
    }
}
