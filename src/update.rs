//! Checks GitHub for a newer release and points the viewer at its page.
//! Nothing is downloaded or installed: without paid signing, a silent
//! self-update would be replacing the app with an unverified binary.
//!
//! Uses the system's curl (part of macOS and Windows 10/11), which saves
//! building a whole TLS stack into a small player for one small request.

use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use eframe::egui;

use crate::i18n::{tf, tr};

/// Redirects to the newest release. Used instead of GitHub's API, which allows
/// 60 requests an hour per IP address: on a shared network or VPN that can
/// already be spent by others, and the check would keep failing.
const LATEST_URL: &str = "https://github.com/calabli/TinyPlayer/releases/latest";
/// Only pages under here are ever opened, whatever the response says.
const TAG_URL: &str = "https://github.com/calabli/TinyPlayer/releases/tag/";
pub const CHECK_EVERY_SECS: i64 = 24 * 60 * 60;
const TIMEOUT_SECS: &str = "15";

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
}

fn numbers(version: &str) -> Vec<u64> {
    version.split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    numbers(candidate) > numbers(current)
}

/// Reads the release out of the redirect `LATEST_URL` answers with. A project
/// with no releases yet redirects to the list instead, which gives None.
fn parse_release(headers: &str) -> Option<Release> {
    let location = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("location").then(|| value.trim())
    })?;
    let version = location.strip_prefix(TAG_URL)?.strip_prefix('v')?;
    let valid = version.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    valid.then(|| Release { version: version.to_string(), url: location.to_string() })
}

fn background(mut cmd: Command) -> Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // Without this, a console window flashes up from the GUI app.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd
}

fn fetch_latest() -> Option<Release> {
    let mut cmd = Command::new("curl");
    cmd.args(["--silent", "--show-error", "--head", "--max-time", TIMEOUT_SECS])
        .args(["--user-agent", &format!("TinyPlayer/{CURRENT_VERSION}")])
        .arg(LATEST_URL);
    let output = background(cmd).output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_release(&String::from_utf8_lossy(&output.stdout))
}

/// Opens a release page in the default browser.
pub fn open_page(url: &str) {
    if !url.starts_with(TAG_URL) {
        return;
    }
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(url);
    let _ = background(cmd).spawn();
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Status {
    Idle,
    Checking,
    UpToDate,
    Found,
    Failed,
}

pub struct UpdateChecker {
    tx: Sender<Option<Release>>,
    rx: Receiver<Option<Release>>,
    status: Status,
}

impl UpdateChecker {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, status: Status::Idle }
    }

    pub fn start(&mut self, ctx: &egui::Context) {
        if self.status == Status::Checking {
            return;
        }
        self.status = Status::Checking;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let _ = tx.send(fetch_latest());
                ctx.request_repaint();
                // The panel's text must not wait for the next input event.
                std::thread::sleep(Duration::from_millis(50));
                ctx.request_repaint();
            })
            .ok();
    }

    /// The outcome of a finished check: `Some(None)` means it failed.
    pub fn poll(&mut self) -> Option<Option<Release>> {
        let result = self.rx.try_recv().ok()?;
        self.status = match &result {
            None => Status::Failed,
            Some(r) if is_newer(&r.version, CURRENT_VERSION) => Status::Found,
            Some(_) => Status::UpToDate,
        };
        Some(result)
    }

    pub fn status_text(&self, latest: Option<&Release>) -> String {
        match (self.status, latest) {
            (Status::Checking, _) => tr("正在检查…").to_string(),
            (Status::UpToDate, _) => tr("已是最新版本").to_string(),
            (Status::Failed, _) => tr("检查失败，请稍后再试").to_string(),
            (Status::Found, Some(r)) => tf("发现新版本 {}", &[&r.version]),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "HTTP/2 302 \r\nserver: GitHub.com\r\nLocation: https://github.com/calabli/TinyPlayer/releases/tag/v0.2.0\r\ncontent-length: 0\r\n\r\n";

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
    }

    #[test]
    fn reads_a_release() {
        let release = parse_release(SAMPLE).unwrap();
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.url, "https://github.com/calabli/TinyPlayer/releases/tag/v0.2.0");
    }

    #[test]
    fn rejects_pages_outside_this_project() {
        assert_eq!(parse_release(&SAMPLE.replace("github.com/calabli", "example.com/calabli")), None);
    }

    #[test]
    fn rejects_odd_tags_and_other_answers() {
        assert_eq!(parse_release(&SAMPLE.replace("v0.2.0", "nightly")), None);
        assert_eq!(parse_release(&SAMPLE.replace("v0.2.0", "v1.x")), None);
        assert_eq!(parse_release("location: https://github.com/calabli/TinyPlayer/releases"), None);
        assert_eq!(parse_release("HTTP/2 200 \r\n\r\n"), None);
        assert_eq!(parse_release(""), None);
    }
}
