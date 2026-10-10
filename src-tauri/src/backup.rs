//! One zip per day in Documents/Dayfile, so the journal survives an uninstall.

use crate::store;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SUPPORTED: bool = cfg!(target_os = "android");
const KEEP_DAYS: usize = 7;

pub trait Sink {
    fn create(&mut self, name: &str, bytes: &[u8]) -> Result<String, String>;
    fn overwrite(&mut self, uri: &str, bytes: &[u8]) -> Result<(), String>;
    fn remove(&mut self, uri: &str);
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Log {
    last: u64,
    error: Option<String>,
    hash: String,
    files: Vec<File>,
}

#[derive(Serialize, Deserialize)]
struct File {
    day: String,
    uri: String,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub last: u64,
    pub error: Option<String>,
}

impl Log {
    fn status(&self) -> Status {
        Status { last: self.last, error: self.error.clone() }
    }
}

fn load(path: &Path) -> Log {
    fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(path: &Path, log: &Log) -> Result<(), String> {
    let text = serde_json::to_string_pretty(log).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("couldn't save backup status: {e}"))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn status(log_path: &Path) -> Status {
    load(log_path).status()
}

pub fn run(dir: &Path, log_path: &Path, day: &str, sink: &mut dyn Sink) -> Result<Status, String> {
    if !store::valid_date(day) {
        return Err("invalid date".into());
    }
    let mut log = load(log_path);
    let outcome = snapshot(dir, &mut log, day, sink);
    match &outcome {
        Ok(true) => {
            log.last = now_ms();
            log.error = None;
        }
        Ok(false) => {}
        Err(e) => log.error = Some(e.clone()),
    }
    save(log_path, &log)?;
    outcome.map(|_| log.status())
}

fn snapshot(dir: &Path, log: &mut Log, day: &str, sink: &mut dyn Sink) -> Result<bool, String> {
    let Some(bytes) = store::backup_bytes(dir)? else { return Ok(false) };
    let hash = store::sha256_hex(&bytes);
    if log.hash == hash && log.files.iter().any(|f| f.day == day) {
        return Ok(true);
    }
    let reused = match log.files.iter().find(|f| f.day == day) {
        Some(f) => sink.overwrite(&f.uri, &bytes).is_ok(),
        None => false,
    };
    if !reused {
        let uri = sink.create(&format!("dayfile-{day}.zip"), &bytes)?;
        log.files.retain(|f| f.day != day);
        log.files.push(File { day: day.to_string(), uri });
    }
    log.hash = hash;
    while log.files.len() > KEEP_DAYS {
        let old = log.files.remove(0);
        sink.remove(&old.uri);
    }
    Ok(true)
}

#[cfg(target_os = "android")]
pub fn platform_sink() -> impl Sink {
    crate::android::MediaStore
}

#[cfg(not(target_os = "android"))]
pub fn platform_sink() -> impl Sink {
    Unsupported
}

#[cfg(not(target_os = "android"))]
struct Unsupported;

#[cfg(not(target_os = "android"))]
impl Sink for Unsupported {
    fn create(&mut self, _: &str, _: &[u8]) -> Result<String, String> {
        Err("automatic backups are only available on Android".into())
    }
    fn overwrite(&mut self, _: &str, _: &[u8]) -> Result<(), String> {
        Err("automatic backups are only available on Android".into())
    }
    fn remove(&mut self, _: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Fake {
        files: BTreeMap<String, Vec<u8>>,
        next: u32,
        writes: u32,
        fail_overwrite: bool,
    }

    impl Sink for Fake {
        fn create(&mut self, name: &str, bytes: &[u8]) -> Result<String, String> {
            self.next += 1;
            self.writes += 1;
            let uri = format!("content://fake/{}/{name}", self.next);
            self.files.insert(uri.clone(), bytes.to_vec());
            Ok(uri)
        }
        fn overwrite(&mut self, uri: &str, bytes: &[u8]) -> Result<(), String> {
            if self.fail_overwrite {
                return Err("gone".into());
            }
            self.writes += 1;
            self.files.insert(uri.to_string(), bytes.to_vec());
            Ok(())
        }
        fn remove(&mut self, uri: &str) {
            self.files.remove(uri);
        }
    }

    fn setup(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("dayfile-backup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dir = base.join("journal");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("2026-01-01.txt"), "hello").unwrap();
        (dir, base.join("backups.json"))
    }

    #[test]
    fn same_day_overwrites_and_new_days_add_files() {
        let (dir, log) = setup("days");
        let mut sink = Fake::default();
        run(&dir, &log, "2026-01-01", &mut sink).unwrap();
        fs::write(dir.join("2026-01-01.txt"), "hello again").unwrap();
        run(&dir, &log, "2026-01-01", &mut sink).unwrap();
        assert_eq!(sink.files.len(), 1);
        let st = run(&dir, &log, "2026-01-02", &mut sink).unwrap();
        assert_eq!(sink.files.len(), 2);
        assert!(st.last > 0 && st.error.is_none());
    }

    #[test]
    fn keeps_only_the_last_week() {
        let (dir, log) = setup("keep");
        let mut sink = Fake::default();
        for d in 1..=9 {
            run(&dir, &log, &format!("2026-01-0{d}"), &mut sink).unwrap();
        }
        assert_eq!(sink.files.len(), KEEP_DAYS);
        assert!(sink.files.keys().all(|u| !u.ends_with("2026-01-01.zip") && !u.ends_with("2026-01-02.zip")));
    }

    #[test]
    fn a_deleted_snapshot_is_recreated() {
        let (dir, log) = setup("gone");
        let mut sink = Fake::default();
        run(&dir, &log, "2026-01-01", &mut sink).unwrap();
        sink.fail_overwrite = true;
        fs::write(dir.join("2026-01-01.txt"), "edited").unwrap();
        run(&dir, &log, "2026-01-01", &mut sink).unwrap();
        assert_eq!(load(&log).files.len(), 1);
        assert_eq!(sink.next, 2);
    }

    #[test]
    fn failures_are_recorded_for_the_settings_line() {
        let (dir, log) = setup("fail");
        assert!(run(&dir, &log, "2026-01-01", &mut Unsupported).is_err());
        assert!(status(&log).error.is_some());
        assert!(run(&dir, &log, "2026-1-1", &mut Fake::default()).is_err());
    }

    #[test]
    fn an_empty_journal_is_not_backed_up() {
        let (dir, log) = setup("empty");
        fs::remove_file(dir.join("2026-01-01.txt")).unwrap();
        let mut sink = Fake::default();
        run(&dir, &log, "2026-01-01", &mut sink).unwrap();
        assert!(sink.files.is_empty());
    }

    #[test]
    fn unchanged_content_is_not_written_again() {
        let (dir, log) = setup("same");
        let mut sink = Fake::default();
        run(&dir, &log, "2026-01-01", &mut sink).unwrap();
        run(&dir, &log, "2026-01-01", &mut sink).unwrap();
        assert_eq!(sink.writes, 1);
        fs::write(dir.join("2026-01-01.txt"), "changed").unwrap();
        run(&dir, &log, "2026-01-01", &mut sink).unwrap();
        assert_eq!(sink.writes, 2);
    }
}
