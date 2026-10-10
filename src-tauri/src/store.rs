//! One plain-text file per day (YYYY-MM-DD.txt, older .md files still work) with optional front matter for mood, tags and star.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Meta {
    pub mood: Option<u8>,
    pub tags: Vec<String>,
    pub starred: bool,
    pub extra: Vec<String>,
}

impl Meta {
    pub fn is_empty(&self) -> bool {
        self.mood.is_none() && self.tags.is_empty() && !self.starred && self.extra.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub date: String,
    pub body: String,
    pub mood: Option<u8>,
    pub tags: Vec<String>,
    pub starred: bool,
    pub extra: Vec<String>,
    pub ext: String,
    // string: a u64 loses precision in JS
    pub rev: String,
    pub modified: u64,
    pub words: usize,
}

// strict on purpose: the date becomes a file name
pub fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    if !b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()) {
        return false;
    }
    let year: u32 = s[0..4].parse().unwrap_or(0);
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

fn rev_of(raw: &[u8]) -> String {
    let mut h = DefaultHasher::new();
    raw.hash(&mut h);
    format!("{:016x}", h.finish())
}

pub fn split_front(text: &str) -> (Meta, String) {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = t.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return (Meta::default(), String::new());
    };
    if first.trim_end() != "---" {
        return (Meta::default(), t.to_string());
    }

    let mut meta = Meta::default();
    let mut consumed = first.len();
    let mut closed = false;
    for line in lines {
        consumed += line.len();
        let l = line.trim_end();
        if l == "---" {
            closed = true;
            break;
        }
        match l.split_once(':') {
            Some((key, value)) => match key.trim() {
                "mood" => meta.mood = value.trim().parse::<u8>().ok().filter(|m| (1..=5).contains(m)),
                "tags" => {
                    meta.tags = value
                        .split(',')
                        .map(|s| s.trim().trim_start_matches('#').to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
                "starred" => meta.starred = value.trim() == "true",
                _ => meta.extra.push(l.to_string()),
            },
            None if !l.is_empty() => meta.extra.push(l.to_string()),
            None => {}
        }
    }
    if !closed {
        return (Meta::default(), t.to_string());
    }
    (meta, t[consumed..].to_string())
}

pub fn compose(meta: &Meta, body: &str) -> String {
    if meta.is_empty() {
        // a leading `---` would be re-read as front matter
        if body.lines().next().is_some_and(|l| l.trim_end() == "---") {
            return format!("---\n---\n{body}");
        }
        return body.to_string();
    }
    let mut out = String::from("---\n");
    if let Some(m) = meta.mood.filter(|m| (1..=5).contains(m)) {
        out.push_str(&format!("mood: {m}\n"));
    }
    let tags: Vec<String> = meta
        .tags
        .iter()
        .map(|t| t.replace([',', '\n', '\r'], " ").trim().trim_start_matches('#').to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if !tags.is_empty() {
        out.push_str(&format!("tags: {}\n", tags.join(", ")));
    }
    if meta.starred {
        out.push_str("starred: true\n");
    }
    for line in &meta.extra {
        let line = line.trim_end();
        if line != "---" && !line.contains('\n') {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

fn find(dir: &Path, date: &str) -> Option<(PathBuf, String)> {
    for ext in ["txt", "md"] {
        let p = dir.join(format!("{date}.{ext}"));
        if p.is_file() {
            return Some((p, ext.to_string()));
        }
    }
    None
}

fn load(path: &Path, date: &str, ext: &str) -> Result<Entry, String> {
    let raw = fs::read(path).map_err(|e| format!("couldn't read {date}: {e}"))?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (meta, body) = split_front(&text);
    let modified = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok(Entry {
        date: date.to_string(),
        words: body.split_whitespace().count(),
        body,
        mood: meta.mood,
        tags: meta.tags,
        starred: meta.starred,
        extra: meta.extra,
        ext: ext.to_string(),
        rev: rev_of(&raw),
        modified,
    })
}

pub fn list(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut found: BTreeMap<String, (PathBuf, String)> = BTreeMap::new();
    for item in fs::read_dir(dir).map_err(|e| format!("couldn't open the journal folder: {e}"))? {
        let Ok(item) = item else { continue };
        let path = item.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let Some((stem, ext)) = name.rsplit_once('.') else { continue };
        if !valid_date(stem) || !(ext == "md" || ext == "txt") {
            continue;
        }
        // .txt wins over .md
        let keep_old = matches!(found.get(stem), Some((_, old)) if old.as_str() == "txt");
        if !keep_old {
            found.insert(stem.to_string(), (path.clone(), ext.to_string()));
        }
    }
    Ok(found
        .into_iter()
        .filter_map(|(date, (path, ext))| load(&path, &date, &ext).ok())
        .collect())
}

pub fn read(dir: &Path, date: &str) -> Result<Option<Entry>, String> {
    if !valid_date(date) {
        return Err("invalid date".into());
    }
    match find(dir, date) {
        Some((path, ext)) => load(&path, date, &ext).map(Some),
        None => Ok(None),
    }
}

/// Fails with "conflict" or "not-utf8" unless `force`; an empty day deletes its file.
pub fn write(
    dir: &Path,
    date: &str,
    body: &str,
    meta: &Meta,
    expected_rev: Option<&str>,
    force: bool,
) -> Result<Option<Entry>, String> {
    if !valid_date(date) {
        return Err("invalid date".into());
    }
    let existing = find(dir, date);

    if !force {
        let raw = match &existing {
            Some((p, _)) => Some(fs::read(p).map_err(|e| e.to_string())?),
            None => None,
        };
        if raw.as_deref().map(rev_of).as_deref() != expected_rev {
            return Err("conflict".into());
        }
        // reads are lossy, so saving would destroy the original bytes
        if raw.as_deref().is_some_and(|r| std::str::from_utf8(r).is_err()) {
            return Err("not-utf8".into());
        }
    }

    if body.trim().is_empty() && meta.is_empty() {
        if let Some((p, _)) = existing {
            fs::remove_file(p).map_err(|e| format!("couldn't remove the empty entry: {e}"))?;
        }
        return Ok(None);
    }

    let (path, ext) = existing.unwrap_or_else(|| (dir.join(format!("{date}.txt")), "txt".to_string()));
    let tmp = dir.join(format!(".{date}.tmp"));
    fs::write(&tmp, compose(meta, body)).map_err(|e| format!("couldn't save: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("couldn't save: {e}")
    })?;
    load(&path, date, &ext).map(Some)
}

pub fn delete(dir: &Path, date: &str) -> Result<(), String> {
    if !valid_date(date) {
        return Err("invalid date".into());
    }
    for ext in ["md", "txt"] {
        let p = dir.join(format!("{date}.{ext}"));
        if p.is_file() {
            fs::remove_file(p).map_err(|e| format!("couldn't delete: {e}"))?;
        }
    }
    Ok(())
}

pub fn export(dir: &Path, path: &Path, format: &str) -> Result<usize, String> {
    if !matches!(format, "txt" | "md" | "json") {
        return Err(format!("unknown export format: {format}"));
    }
    let entries = list(dir)?;
    let file = fs::File::create(path).map_err(|e| format!("couldn't write the export: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for e in &entries {
        let text = match format {
            "json" => serde_json::to_string_pretty(e).map_err(|err| err.to_string())?,
            "md" => entry_markdown(e),
            _ => compose(
                &Meta { mood: e.mood, tags: e.tags.clone(), starred: e.starred, extra: e.extra.clone() },
                &e.body,
            ),
        };
        zip.start_file(format!("{}.{format}", e.date), opts).map_err(|e| format!("couldn't write the export: {e}"))?;
        zip.write_all(text.as_bytes()).map_err(|e| format!("couldn't write the export: {e}"))?;
    }
    zip.finish().map_err(|e| format!("couldn't write the export: {e}"))?;
    Ok(entries.len())
}

fn entry_markdown(e: &Entry) -> String {
    let mut out = format!("## {}", e.date);
    if e.starred {
        out.push_str(" ★");
    }
    if let Some(m) = e.mood {
        out.push_str(&format!(" · mood {m}/5"));
    }
    if !e.tags.is_empty() {
        let tags: Vec<String> = e.tags.iter().map(|t| format!("#{t}")).collect();
        out.push_str(&format!(" · {}", tags.join(" ")));
    }
    out.push_str("\n\n");
    out.push_str(e.body.trim_end());
    out.push('\n');
    out
}

pub fn ensure_writable(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("couldn't use that folder: {e}"))?;
    let probe = dir.join(".dayfile-write-test");
    fs::write(&probe, b"ok").map_err(|e| format!("that folder isn't writable: {e}"))?;
    let _ = fs::remove_file(probe);
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Config {
    dir: String,
}

pub fn load_dir(cfg: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(cfg).ok()?;
    let c: Config = serde_json::from_str(&text).ok()?;
    let p = PathBuf::from(c.dir);
    p.is_dir().then_some(p)
}

pub fn save_dir(cfg: &Path, dir: &Path) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&Config { dir: dir.to_string_lossy().into_owned() })
        .map_err(|e| e.to_string())?;
    fs::write(cfg, text).map_err(|e| format!("couldn't save settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("dayfile-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn dates() {
        assert!(valid_date("2026-10-09"));
        assert!(!valid_date("2026-13-01"));
        assert!(!valid_date("../../etc/p"));
        assert!(!valid_date("2026-1-009"));
        assert!(!valid_date("2026-10-0é"));
    }

    #[test]
    fn plain_text_has_no_front_matter() {
        let (meta, body) = split_front("just words\nmore");
        assert!(meta.is_empty());
        assert_eq!(body, "just words\nmore");
        assert_eq!(compose(&meta, &body), "just words\nmore");
    }

    #[test]
    fn front_matter_round_trip_keeps_unknown_lines() {
        let src = "---\nmood: 4\ntags: a, #b\nstarred: true\naliases: x\n---\nhello\n\nworld";
        let (meta, body) = split_front(src);
        assert_eq!(meta.mood, Some(4));
        assert_eq!(meta.tags, vec!["a", "b"]);
        assert!(meta.starred);
        assert_eq!(meta.extra, vec!["aliases: x"]);
        assert_eq!(body, "hello\n\nworld");
        let again = compose(&meta, &body);
        assert_eq!(split_front(&again), (meta, body));
    }

    #[test]
    fn unclosed_front_matter_is_just_text() {
        let (meta, body) = split_front("---\nnot metadata");
        assert!(meta.is_empty());
        assert_eq!(body, "---\nnot metadata");
    }

    #[test]
    fn write_detects_external_change() {
        let dir = scratch("conflict");
        let meta = Meta::default();
        let first = write(&dir, "2026-10-09", "one", &meta, None, false).unwrap().unwrap();
        fs::write(dir.join("2026-10-09.txt"), "edited elsewhere").unwrap();
        let err = write(&dir, "2026-10-09", "two", &meta, Some(&first.rev), false).unwrap_err();
        assert_eq!(err, "conflict");
        let forced = write(&dir, "2026-10-09", "two", &meta, Some(&first.rev), true).unwrap().unwrap();
        assert_eq!(forced.body, "two");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn empty_entry_removes_file_and_txt_is_kept() {
        let dir = scratch("empty");
        fs::write(dir.join("2026-01-02.txt"), "old journal").unwrap();
        let e = read(&dir, "2026-01-02").unwrap().unwrap();
        assert_eq!(e.ext, "txt");
        let saved = write(&dir, "2026-01-02", "new text", &Meta::default(), Some(&e.rev), false)
            .unwrap()
            .unwrap();
        assert_eq!(saved.ext, "txt");
        assert!(write(&dir, "2026-01-02", "  \n", &Meta::default(), Some(&saved.rev), false)
            .unwrap()
            .is_none());
        assert!(read(&dir, "2026-01-02").unwrap().is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn dates_must_exist() {
        assert!(valid_date("2024-02-29"));
        assert!(!valid_date("2026-02-29"));
        assert!(!valid_date("2026-04-31"));
        assert!(!valid_date("1900-02-29"));
        assert!(valid_date("2000-02-29"));
    }

    #[test]
    fn body_starting_with_a_rule_survives() {
        let body = "---\nfeeling odd today\n---\nreal text";
        let (meta, parsed) = split_front(&compose(&Meta::default(), body));
        assert!(meta.is_empty());
        assert_eq!(parsed, body);
    }

    #[test]
    fn non_utf8_file_is_not_overwritten_without_force() {
        let dir = scratch("latin1");
        fs::write(dir.join("2020-05-05.txt"), b"caf\xe9 au lait").unwrap();
        let e = read(&dir, "2020-05-05").unwrap().unwrap();
        let err = write(&dir, "2020-05-05", "x", &Meta::default(), Some(&e.rev), false).unwrap_err();
        assert_eq!(err, "not-utf8");
        assert_eq!(fs::read(dir.join("2020-05-05.txt")).unwrap(), b"caf\xe9 au lait");
        assert!(write(&dir, "2020-05-05", "x", &Meta::default(), Some(&e.rev), true).is_ok());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn new_entries_are_txt_and_old_md_stays_md() {
        let dir = scratch("ext");
        let made = write(&dir, "2026-10-09", "fresh", &Meta::default(), None, false).unwrap().unwrap();
        assert_eq!(made.ext, "txt");
        assert!(dir.join("2026-10-09.txt").is_file() && !dir.join("2026-10-09.md").exists());

        fs::write(dir.join("2026-10-08.md"), "legacy").unwrap();
        let old = read(&dir, "2026-10-08").unwrap().unwrap();
        let saved = write(&dir, "2026-10-08", "legacy edited", &Meta::default(), Some(&old.rev), false).unwrap().unwrap();
        assert_eq!(saved.ext, "md");
        assert!(!dir.join("2026-10-08.txt").exists());

        fs::write(dir.join("2026-10-07.md"), "from md").unwrap();
        fs::write(dir.join("2026-10-07.txt"), "from txt").unwrap();
        assert_eq!(read(&dir, "2026-10-07").unwrap().unwrap().body, "from txt");
        let listed = list(&dir).unwrap();
        assert_eq!(listed.iter().find(|e| e.date == "2026-10-07").unwrap().body, "from txt");
        let _ = fs::remove_dir_all(dir);
    }
}
