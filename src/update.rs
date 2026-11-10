//! In-app updates: looks for a newer release on GitHub, downloads it, checks
//! it and installs it, with the usual dialog steps around that. A build of
//! the same version from a different commit counts as out of date too.
//!
//! The installer can't replace a running app, so a small script waits for this
//! process to exit and does the swap. On macOS it keeps the old app until the
//! new one is fully copied, and puts it back if that fails.
//!
//! Uses the system's curl (part of macOS and Windows 10/11), which saves
//! building a whole TLS stack into a small player for a few requests.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use eframe::egui;
use sha2::{Digest, Sha256};

use crate::i18n::{tf, tr};

/// Redirects to the newest release. Used instead of GitHub's API, which allows
/// 60 requests an hour per IP address: on a shared network or VPN that can
/// already be spent by others, and the check would keep failing.
const RELEASES_URL: &str = "https://github.com/calabli/TinyPlayer/releases";
const RAW_URL: &str = "https://raw.githubusercontent.com/calabli/TinyPlayer";
pub const CHECK_EVERY_SECS: i64 = 24 * 60 * 60;
const CHECK_TIMEOUT_SECS: &str = "15";
const DOWNLOAD_TIMEOUT_SECS: &str = "3600";
const SUMS_NAME: &str = "SHA256SUMS";
/// Holds the commit a release was built from.
const COMMIT_NAME: &str = "COMMIT";
/// The dialog only has room for the highlights.
const MAX_NOTE_LINES: usize = 8;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Set by GitHub Actions. Local builds don't have one, and go by version.
const CURRENT_COMMIT: Option<&str> = option_env!("GITHUB_SHA");

fn numbers(version: &str) -> Vec<u64> {
    version.split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    numbers(candidate) > numbers(current)
}

/// Whether to offer `latest`. Builds of the same version are told apart by
/// their commit, where both are known; a build ahead of it is left alone.
fn is_outdated(latest: &str, current: &str, latest_commit: Option<&str>, own_commit: Option<&str>) -> bool {
    if is_newer(current, latest) {
        return false;
    }
    is_newer(latest, current) || latest_commit.zip(own_commit).is_some_and(|(a, b)| !a.eq_ignore_ascii_case(b))
}

/// A COMMIT file's content, if it holds a full commit hash.
fn parse_commit(text: &str) -> Option<&str> {
    let text = text.trim();
    (text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit())).then_some(text)
}

/// Reads the version out of the redirect `releases/latest` answers with. A
/// project with no releases yet redirects to the list instead, giving None.
fn parse_latest(headers: &str) -> Option<String> {
    let location = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("location").then(|| value.trim())
    })?;
    let version = location.strip_prefix(RELEASES_URL)?.strip_prefix("/tag/v")?;
    let valid = version.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    valid.then(|| version.to_string())
}

/// The bullet points of `version` in CHANGELOG.md.
fn parse_notes(changelog: &str, version: &str) -> String {
    let heading = format!("## {version}");
    let lines = changelog
        .lines()
        .skip_while(|l| l.trim() != heading)
        .skip(1)
        .take_while(|l| !l.starts_with("## "))
        .filter(|l| !l.trim().is_empty());
    lines.take(MAX_NOTE_LINES).collect::<Vec<_>>().join("\n")
}

/// The expected hash of `name` in a SHA256SUMS file.
fn parse_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*');
        (file == name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

/// The package this build updates from; None where none is published.
fn asset_name(version: &str) -> Option<String> {
    let suffix = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-arm64.dmg"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x64-setup.exe"
    } else {
        return None;
    };
    Some(format!("TinyPlayer-{version}-{suffix}"))
}

fn background(mut cmd: Command) -> Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // Without this, a console window flashes up from the GUI app.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(Stdio::null());
    cmd
}

fn curl(timeout: &str) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args(["--silent", "--show-error", "--max-time", timeout])
        .args(["--user-agent", &format!("TinyPlayer/{CURRENT_VERSION}")]);
    background(cmd)
}

/// Running curl processes, so none is left behind when the app quits.
static RUNNING: Mutex<Vec<Child>> = Mutex::new(Vec::new());

enum Run {
    Going,
    Finished(bool),
    Stopped,
}

fn track(child: Child) -> u32 {
    let pid = child.id();
    if let Ok(mut running) = RUNNING.lock() {
        running.push(child);
    }
    pid
}

// A child is dropped only after `try_wait` has collected it, or has failed
// (so there's nothing more to collect).
#[allow(clippy::zombie_processes)]
fn check(pid: u32) -> Run {
    let Ok(mut running) = RUNNING.lock() else { return Run::Stopped };
    let Some(at) = running.iter().position(|c| c.id() == pid) else { return Run::Stopped };
    match running[at].try_wait() {
        Ok(None) => Run::Going,
        Ok(Some(status)) => {
            running.remove(at);
            Run::Finished(status.success())
        }
        Err(_) => {
            running.remove(at);
            Run::Stopped
        }
    }
}

fn kill(pid: u32) {
    let Ok(mut running) = RUNNING.lock() else { return };
    if let Some(at) = running.iter().position(|c| c.id() == pid) {
        let mut child = running.remove(at);
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Stops every download and check. The install script isn't one of these: it
/// has to outlive the app to replace it.
pub fn stop_all() {
    if let Ok(mut running) = RUNNING.lock() {
        for child in running.iter_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        running.clear();
    }
}

/// Runs curl and returns what it printed, if it worked.
fn capture(mut cmd: Command) -> Option<Vec<u8>> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let pid = track(child);
    let worked = loop {
        match check(pid) {
            Run::Going => std::thread::sleep(Duration::from_millis(20)),
            Run::Finished(worked) => break worked,
            Run::Stopped => break false,
        }
    };
    let bytes = reader.join().ok()?;
    worked.then_some(bytes)
}

fn fetch_latest() -> Option<String> {
    let mut cmd = curl(CHECK_TIMEOUT_SECS);
    cmd.arg("--head").arg(format!("{RELEASES_URL}/latest"));
    parse_latest(&String::from_utf8_lossy(&capture(cmd)?))
}

fn fetch_text(url: &str) -> Option<String> {
    let mut cmd = curl(CHECK_TIMEOUT_SECS);
    cmd.args(["--fail", "--location", url]);
    Some(String::from_utf8_lossy(&capture(cmd)?).into_owned())
}

/// Only asks for the commit when it can make a difference.
fn outdated(latest: &str) -> bool {
    let commit = CURRENT_COMMIT.filter(|_| latest == CURRENT_VERSION).and_then(|_| {
        let text = fetch_text(&format!("{RELEASES_URL}/download/v{latest}/{COMMIT_NAME}"))?;
        parse_commit(&text).map(str::to_string)
    });
    is_outdated(latest, CURRENT_VERSION, commit.as_deref(), CURRENT_COMMIT)
}

fn download_size(url: &str) -> u64 {
    let mut cmd = curl(CHECK_TIMEOUT_SECS);
    cmd.args(["--head", "--location", url]);
    let Some(output) = capture(cmd) else { return 0 };
    // Each redirect has its own headers; the last answer is the file.
    String::from_utf8_lossy(&output)
        .lines()
        .filter_map(|l| l.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .filter_map(|(_, value)| value.trim().parse().ok())
        .next_back()
        .unwrap_or(0)
}

fn sha256_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Where an update can go, or None for a portable copy, a development build,
/// or a read-only location, which can't update themselves.
fn install_target() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if cfg!(target_os = "windows") {
        let dir = exe.parent()?;
        return dir.join("unins000.exe").exists().then(|| exe.clone());
    }
    let app = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"))?.to_path_buf();
    let text = app.to_string_lossy();
    // Running from the disk image, or from macOS's quarantine copy of the app.
    if text.starts_with("/Volumes/") || text.contains("/AppTranslocation/") {
        return None;
    }
    let probe = app.parent()?.join(".tinyplayer-update-probe");
    std::fs::write(&probe, b"").ok()?;
    let _ = std::fs::remove_file(probe);
    Some(app)
}

pub fn can_update() -> bool {
    install_target().is_some() && asset_name(CURRENT_VERSION).is_some()
}

fn updates_dir() -> PathBuf {
    crate::history::data_dir().join("updates")
}

/// A downloaded, checked update, waiting to be installed.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub version: String,
    /// The staged app on macOS, the installer on Windows.
    path: PathBuf,
}

enum Msg {
    /// The latest version, and whether this build should update to it.
    Checked(Option<(String, bool)>),
    Notes(String),
    Progress(u64, u64),
    Prepared(Prepared),
    Failed(&'static str),
    Cancelled,
}

fn download(version: &str, tx: &Sender<Msg>, cancel: &AtomicBool) -> Result<Prepared, Option<&'static str>> {
    const NETWORK: &str = "Download failed. Check your network";
    let name = asset_name(version).ok_or(Some("No update package for this system"))?;
    let base = format!("{RELEASES_URL}/download/v{version}");
    let dir = updates_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|_| Some("Couldn't write the update files"))?;
    let sums = fetch_text(&format!("{base}/{SUMS_NAME}")).ok_or(Some("Update checksum unavailable"))?;
    let expected = parse_sum(&sums, &name).ok_or(Some("Update checksum unavailable"))?;

    let url = format!("{base}/{name}");
    let total = download_size(&url);
    let file = dir.join(&name);
    let child = curl(DOWNLOAD_TIMEOUT_SECS)
        .args(["--fail", "--location", "--output"])
        .arg(&file)
        .arg(&url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Some(NETWORK))?;
    let pid = track(child);
    loop {
        if cancel.load(Ordering::Relaxed) {
            kill(pid);
            let _ = std::fs::remove_dir_all(&dir);
            return Err(None);
        }
        match check(pid) {
            Run::Finished(true) => break,
            Run::Finished(false) | Run::Stopped => return Err(Some(NETWORK)),
            Run::Going => {}
        }
        let done = std::fs::metadata(&file).map_or(0, |m| m.len());
        let _ = tx.send(Msg::Progress(done, total));
        std::thread::sleep(Duration::from_millis(200));
    }
    if sha256_file(&file).as_deref() != Some(expected.as_str()) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(Some("The download failed its check and was deleted"));
    }
    let path = stage(&file, &dir).ok_or(Some("Couldn't prepare the update"))?;
    Ok(Prepared { version: version.to_string(), path })
}

#[cfg(target_os = "macos")]
fn stage(dmg: &Path, dir: &Path) -> Option<PathBuf> {
    let mount = dir.join("mount");
    let staged = dir.join("TinyPlayer.app");
    let status = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
        .arg(&mount)
        .arg(dmg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let copied = Command::new("ditto").arg(mount.join("TinyPlayer.app")).arg(&staged).status();
    let _ = Command::new("hdiutil").arg("detach").arg(&mount).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = std::fs::remove_file(dmg);
    let verified = Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(&staged).stderr(Stdio::null()).status();
    (copied.ok()?.success() && verified.ok()?.success()).then_some(staged)
}

#[cfg(not(target_os = "macos"))]
fn stage(installer: &Path, _dir: &Path) -> Option<PathBuf> {
    Some(installer.to_path_buf())
}

const MAC_INSTALL: &str = r#"#!/bin/sh
pid=$1; staged=$2; app=$3; relaunch=$4; dir=$5
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
old="$app.old.$$"
if mv "$app" "$old"; then
    if ditto "$staged" "$app"; then
        rm -rf "$old"
    else
        rm -rf "$app"
        mv "$old" "$app"
    fi
fi
if [ "$relaunch" = 1 ]; then open "$app"; fi
rm -rf "$dir"
"#;

const WIN_INSTALL: &str = "@echo off\r
:wait\r
tasklist /FI \"PID eq %1\" | find \"%1\" >nul && (timeout /t 1 /nobreak >nul & goto wait)\r
\"%~2\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART\r
if \"%~4\"==\"1\" start \"\" \"%~3\"\r
";

/// Starts the script that installs after this process exits, and returns
/// false if it couldn't. The caller then has to exit for it to go ahead.
pub fn install(prepared: &Prepared, relaunch: bool) -> bool {
    let Some(target) = install_target() else { return false };
    let dir = updates_dir();
    let pid = std::process::id().to_string();
    let flag = if relaunch { "1" } else { "0" };
    let cmd = if cfg!(target_os = "windows") {
        let script = dir.join("install.cmd");
        if std::fs::write(&script, WIN_INSTALL).is_err() {
            return false;
        }
        let mut c = Command::new("cmd");
        c.arg("/c").arg(script).arg(pid).arg(&prepared.path).arg(&target).arg(flag);
        c
    } else {
        let script = dir.join("install.sh");
        if std::fs::write(&script, MAC_INSTALL).is_err() {
            return false;
        }
        let mut c = Command::new("/bin/sh");
        c.arg(script).arg(pid).arg(&prepared.path).arg(&target).arg(flag).arg(&dir);
        c
    };
    background(cmd).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Idle,
    Checking,
    UpToDate,
    CheckFailed,
    Available(String),
    Downloading { version: String, done: u64, total: u64 },
    Ready(String),
    Failed { version: String, reason: &'static str },
}

/// What the update dialog shows.
pub enum Dialog<'a> {
    Available { version: &'a str, notes: &'a str, can_update: bool },
    Downloading { version: &'a str, done: u64, total: u64 },
    Ready { version: &'a str },
    Failed { reason: &'static str },
}

pub struct Updater {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    phase: Phase,
    notes: String,
    cancel: Arc<AtomicBool>,
    prepared: Option<Prepared>,
    /// Install when the app exits, whatever the viewer does until then.
    pub install_on_exit: bool,
}

impl Updater {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            phase: Phase::Idle,
            notes: String::new(),
            cancel: Arc::default(),
            prepared: None,
            install_on_exit: false,
        }
    }

    pub fn check(&mut self, ctx: &egui::Context) {
        if matches!(self.phase, Phase::Checking | Phase::Downloading { .. } | Phase::Ready(_)) {
            return;
        }
        self.phase = Phase::Checking;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        let english = crate::i18n::is_english();
        std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let latest = fetch_latest().map(|version| {
                    let update = outdated(&version);
                    (version, update)
                });
                let _ = tx.send(Msg::Checked(latest.clone()));
                ctx.request_repaint();
                let Some((version, update)) = latest else { return };
                if update {
                    // The English changelog is new in 0.1.21; older releases
                    // only have the Chinese one.
                    let files: &[&str] = if english { &["CHANGELOG.en.md", "CHANGELOG.md"] } else { &["CHANGELOG.md"] };
                    let notes = files.iter().find_map(|file| {
                        let text = fetch_text(&format!("{RAW_URL}/v{version}/{file}"))?;
                        Some(parse_notes(&text, &version)).filter(|n| !n.is_empty())
                    });
                    if let Some(notes) = notes {
                        let _ = tx.send(Msg::Notes(notes));
                        ctx.request_repaint();
                    }
                }
            })
            .ok();
    }

    pub fn start_download(&mut self, ctx: &egui::Context) {
        let version = match &self.phase {
            Phase::Available(v) => v.clone(),
            Phase::Failed { version, .. } => version.clone(),
            _ => return,
        };
        self.phase = Phase::Downloading { version: version.clone(), done: 0, total: 0 };
        self.cancel = Arc::default();
        let (tx, ctx, cancel) = (self.tx.clone(), ctx.clone(), self.cancel.clone());
        std::thread::Builder::new()
            .name("update-download".into())
            .spawn(move || {
                let msg = match download(&version, &tx, &cancel) {
                    Ok(prepared) => Msg::Prepared(prepared),
                    Err(Some(reason)) => Msg::Failed(reason),
                    Err(None) => Msg::Cancelled,
                };
                let _ = tx.send(msg);
                ctx.request_repaint();
            })
            .ok();
    }

    pub fn cancel_download(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Back to not offering anything, after "later" or "skip".
    pub fn dismiss(&mut self) {
        self.phase = Phase::Idle;
    }

    /// Applies finished background work. Returns the version a check found,
    /// when this build should update to it.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<PollResult> {
        let mut result = None;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Checked(None) => {
                    self.phase = Phase::CheckFailed;
                    result = Some(PollResult::CheckFinished(None));
                }
                Msg::Checked(Some((version, update))) => {
                    self.phase = if update { Phase::Available(version.clone()) } else { Phase::UpToDate };
                    result = Some(PollResult::CheckFinished(update.then_some(version)));
                }
                Msg::Notes(notes) => self.notes = notes,
                Msg::Progress(done, total) => {
                    if let Phase::Downloading { done: d, total: t, .. } = &mut self.phase {
                        (*d, *t) = (done, total);
                        ctx.request_repaint();
                    }
                }
                Msg::Prepared(prepared) => {
                    self.phase = Phase::Ready(prepared.version.clone());
                    self.prepared = Some(prepared);
                    result = Some(PollResult::Ready);
                }
                Msg::Failed(reason) => {
                    let version = match &self.phase {
                        Phase::Downloading { version, .. } => version.clone(),
                        _ => String::new(),
                    };
                    self.phase = Phase::Failed { version, reason };
                    result = Some(PollResult::Failed);
                }
                Msg::Cancelled => self.phase = Phase::Idle,
            }
        }
        result
    }

    pub fn dialog(&self) -> Option<Dialog<'_>> {
        Some(match &self.phase {
            Phase::Available(version) => Dialog::Available { version, notes: &self.notes, can_update: can_update() },
            Phase::Downloading { version, done, total } => Dialog::Downloading { version, done: *done, total: *total },
            Phase::Ready(version) => Dialog::Ready { version },
            Phase::Failed { reason, .. } => Dialog::Failed { reason },
            _ => return None,
        })
    }

    /// Hands over the update to install; a second call gets nothing, so it
    /// can't be installed twice.
    pub fn take_prepared(&mut self) -> Option<Prepared> {
        self.prepared.take()
    }

    pub fn status_text(&self) -> String {
        match &self.phase {
            Phase::Idle => String::new(),
            Phase::Checking => tr("Checking…").to_string(),
            Phase::UpToDate => tr("You're up to date").to_string(),
            Phase::CheckFailed => tr("Couldn't check. Try again later").to_string(),
            Phase::Available(v) => tf("Version {} is available", &[v]),
            Phase::Downloading { version, .. } => tf("Downloading {}", &[version]),
            Phase::Ready(v) => tf("Version {} is ready", &[v]),
            Phase::Failed { reason, .. } => tr(reason).to_string(),
        }
    }
}

pub enum PollResult {
    /// A check ended; holds the version if this build should update to it.
    CheckFinished(Option<String>),
    Ready,
    Failed,
}

/// Leftovers of an update that already happened or was abandoned.
pub fn clean_up() {
    let _ = std::fs::remove_dir_all(updates_dir());
}

#[cfg(test)]
mod tests {
    use super::*;

    const REDIRECT: &str = "HTTP/2 302 \r\nserver: GitHub.com\r\nLocation: https://github.com/calabli/TinyPlayer/releases/tag/v0.2.0\r\ncontent-length: 0\r\n\r\n";

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
    }

    #[test]
    fn a_different_commit_of_the_same_version_is_outdated() {
        let (a, b) = (Some("a".repeat(40)), Some("b".repeat(40)));
        let (a, b) = (a.as_deref(), b.as_deref());
        assert!(is_outdated("0.2.0", "0.2.0", a, b));
        assert!(!is_outdated("0.2.0", "0.2.0", a, a));
        assert!(!is_outdated("0.2.0", "0.2.0", Some(&"A".repeat(40)), a));
    }

    #[test]
    fn versions_decide_when_a_commit_is_unknown() {
        let a = Some("a".repeat(40));
        assert!(is_outdated("0.2.0", "0.1.0", None, None));
        assert!(!is_outdated("0.2.0", "0.2.0", None, a.as_deref()));
        assert!(!is_outdated("0.2.0", "0.2.0", a.as_deref(), None));
    }

    #[test]
    fn a_build_ahead_of_the_release_is_left_alone() {
        let (a, b) = (Some("a".repeat(40)), Some("b".repeat(40)));
        assert!(!is_outdated("0.2.0", "0.3.0", a.as_deref(), b.as_deref()));
    }

    #[test]
    fn reads_only_full_commit_hashes() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(parse_commit(&format!("{hash}\n")), Some(hash));
        assert_eq!(parse_commit("abc123"), None);
        assert_eq!(parse_commit(&"z".repeat(40)), None);
        assert_eq!(parse_commit("<html>not found</html>"), None);
    }

    #[test]
    fn reads_the_latest_version() {
        assert_eq!(parse_latest(REDIRECT).as_deref(), Some("0.2.0"));
    }

    #[test]
    fn rejects_other_sites_and_odd_tags() {
        assert_eq!(parse_latest(&REDIRECT.replace("github.com/calabli", "example.com/calabli")), None);
        assert_eq!(parse_latest(&REDIRECT.replace("v0.2.0", "nightly")), None);
        assert_eq!(parse_latest(&REDIRECT.replace("v0.2.0", "v1.x")), None);
        assert_eq!(parse_latest("location: https://github.com/calabli/TinyPlayer/releases"), None);
        assert_eq!(parse_latest("HTTP/2 200 \r\n\r\n"), None);
    }

    #[test]
    fn both_changelogs_cover_the_same_versions() {
        let versions = |text: &str| text.lines().filter_map(|l| l.strip_prefix("## ")).map(str::to_string).collect::<Vec<_>>();
        let (zh, en) = (include_str!("../CHANGELOG.md"), include_str!("../CHANGELOG.en.md"));
        assert_eq!(versions(zh), versions(en));
        for v in versions(en) {
            assert_eq!(parse_notes(zh, &v).lines().count(), parse_notes(en, &v).lines().count(), "{v}");
        }
    }

    #[test]
    fn notes_come_from_the_right_version_and_are_limited() {
        let log = "# Changes\n\n## 0.3.0\n\n- new\n\n## 0.2.0\n\n- one\n- two\n\n## 0.1.0\n\n- old\n";
        assert_eq!(parse_notes(log, "0.2.0"), "- one\n- two");
        assert_eq!(parse_notes(log, "9.9.9"), "");
        let long = format!("## 1.0\n{}", "- x\n".repeat(30));
        assert_eq!(parse_notes(&long, "1.0").lines().count(), MAX_NOTE_LINES);
    }

    #[test]
    fn finds_a_file_in_a_checksum_list() {
        let hash = "a".repeat(64);
        let sums = format!("{hash}  TinyPlayer.dmg\n{}  other.zip\n", "b".repeat(64));
        assert_eq!(parse_sum(&sums, "TinyPlayer.dmg"), Some(hash.clone()));
        assert_eq!(parse_sum(&format!("{hash} *setup.exe"), "setup.exe"), Some(hash));
        assert_eq!(parse_sum(&sums, "missing"), None);
        assert_eq!(parse_sum("short  x", "x"), None);
    }

    #[test]
    fn hashes_a_file_like_sha256sum() {
        let path = std::env::temp_dir().join(format!("tinyplayer-hash-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_failed_check_can_be_retried_and_a_download_ignores_checks() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        u.phase = Phase::Downloading { version: "1.0".into(), done: 0, total: 0 };
        u.check(&ctx);
        assert!(matches!(u.phase, Phase::Downloading { .. }));
        u.phase = Phase::Failed { version: "1.0".into(), reason: "x" };
        assert!(matches!(u.dialog(), Some(Dialog::Failed { .. })));
        u.dismiss();
        assert!(u.dialog().is_none());
    }
}
