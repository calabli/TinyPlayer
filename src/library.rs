//! Media library: watched folders, the videos found in them, and tags.
//! Shares the SQLite file with `history.rs` (separate connection, WAL mode).
//!
//! Search uses `LIKE` rather than FTS5: SQLite's FTS tokenizers don't split
//! Chinese text, so "王朝" wouldn't match "大明王朝". Filtering a few thousand
//! rows in memory is well under a frame anyway.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::history::now_secs;

#[derive(Clone, Debug)]
pub struct Folder {
    pub id: i64,
    pub path: String,
    pub count: i64,
    /// Videos that couldn't be found, such as on an unplugged disk.
    pub missing: i64,
}

#[derive(Clone, Debug)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

#[derive(Clone, Debug)]
pub struct Video {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub folder_id: i64,
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub thumb: Option<String>,
    pub added_at: i64,
    /// Saved resume position, if any (from `history`).
    pub position: Option<f64>,
    pub last_played: Option<i64>,
    pub plays: i64,
    pub tags: Vec<String>,
    /// Ids of the collections (favorites lists) this video is in.
    pub collections: Vec<i64>,
    pub size: i64,
    pub mtime: i64,
}

impl Video {
    pub fn progress(&self) -> Option<f32> {
        match self.position {
            Some(p) if self.duration > 0.0 => Some((p / self.duration).clamp(0.0, 1.0) as f32),
            _ => None,
        }
    }
}

/// A named group of watched folders.
#[derive(Clone, Debug)]
pub struct FolderSet {
    pub id: i64,
    pub name: String,
    pub folders: Vec<i64>,
}

pub struct Library {
    db: Connection,
}

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS folders (
        id       INTEGER PRIMARY KEY,
        path     TEXT NOT NULL UNIQUE,
        added_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS videos (
        id        INTEGER PRIMARY KEY,
        folder_id INTEGER NOT NULL,
        path      TEXT NOT NULL UNIQUE,
        key       TEXT NOT NULL,
        title     TEXT NOT NULL,
        size      INTEGER NOT NULL,
        mtime     INTEGER NOT NULL,
        duration  REAL NOT NULL DEFAULT 0,
        width     INTEGER NOT NULL DEFAULT 0,
        height    INTEGER NOT NULL DEFAULT 0,
        thumb     TEXT,
        probed    INTEGER NOT NULL DEFAULT 0,   -- 0 = pending, 1 = done, 2 = failed
        missing   INTEGER NOT NULL DEFAULT 0,
        added_at  INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS videos_folder ON videos(folder_id);
    CREATE INDEX IF NOT EXISTS videos_key ON videos(key);
    CREATE TABLE IF NOT EXISTS tags (
        id   INTEGER PRIMARY KEY,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE
    );
    CREATE TABLE IF NOT EXISTS collections (
        id         INTEGER PRIMARY KEY,
        name       TEXT NOT NULL,
        sort_order INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS collection_items (
        collection_id INTEGER NOT NULL,
        video_id      INTEGER NOT NULL,
        added_at      INTEGER NOT NULL,
        PRIMARY KEY (collection_id, video_id)
    );
    CREATE TABLE IF NOT EXISTS folder_sets (
        id         INTEGER PRIMARY KEY,
        name       TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS folder_set_items (
        set_id    INTEGER NOT NULL,
        folder_id INTEGER NOT NULL,
        PRIMARY KEY (set_id, folder_id)
    );
    CREATE TABLE IF NOT EXISTS dup_ignored (key TEXT PRIMARY KEY);
    CREATE TABLE IF NOT EXISTS video_tags (
        video_id INTEGER NOT NULL,
        tag_id   INTEGER NOT NULL,
        PRIMARY KEY (video_id, tag_id)
    );
";

impl Library {
    pub fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(3)).map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
            .map_err(|e| e.to_string())?;
        // history.rs owns these tables; make sure they exist for the joins below.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (key TEXT PRIMARY KEY, path TEXT NOT NULL,
                 title TEXT NOT NULL, position REAL NOT NULL, duration REAL NOT NULL,
                 updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS play_events (id INTEGER PRIMARY KEY, key TEXT NOT NULL,
                 path TEXT NOT NULL, started_at INTEGER NOT NULL, watched_secs REAL NOT NULL DEFAULT 0);",
        )
        .map_err(|e| e.to_string())?;
        db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Self { db })
    }

    pub fn db_path() -> PathBuf {
        crate::history::data_dir().join("tinyplayer.db")
    }

    pub fn thumbs_dir() -> PathBuf {
        crate::history::data_dir().join("thumbs")
    }

    // ---- Folders ----

    /// Adds a watched folder. Returns its id (existing id if already added).
    /// Groups writes until [`Self::commit`], which is far faster than
    /// committing each of thousands on its own.
    pub fn begin(&self) {
        let _ = self.db.execute_batch("BEGIN IMMEDIATE");
    }

    pub fn commit(&self) {
        let _ = self.db.execute_batch("COMMIT");
    }

    /// Forgets the whole library, in one step. Play history stays.
    pub fn clear(&self) {
        self.begin();
        for table in ["video_tags", "tags", "collection_items", "folder_set_items", "folder_sets", "dup_ignored", "videos", "folders"] {
            let _ = self.db.execute(&format!("DELETE FROM {table}"), []);
        }
        self.commit();
    }

    /// Drops rows a scan added for a folder that's since been removed.
    pub fn remove_orphans(&self) {
        let _ = self.db.execute("DELETE FROM videos WHERE folder_id NOT IN (SELECT id FROM folders)", []);
    }

    pub fn add_folder(&self, path: &str) -> Option<i64> {
        let canon = std::fs::canonicalize(path).ok()?;
        let p = display_path(&canon);
        let _ = self.db.execute(
            "INSERT OR IGNORE INTO folders (path, added_at) VALUES (?1, ?2)",
            params![p, now_secs()],
        );
        self.db.query_row("SELECT id FROM folders WHERE path = ?1", [&p], |r| r.get(0)).ok()
    }

    /// Removes a folder and its videos from the library (files are untouched).
    pub fn remove_folder(&self, id: i64) {
        let _ = self.db.execute(
            "DELETE FROM video_tags WHERE video_id IN (SELECT id FROM videos WHERE folder_id = ?1)",
            [id],
        );
        let _ = self.db.execute("DELETE FROM videos WHERE folder_id = ?1", [id]);
        let _ = self.db.execute("DELETE FROM folders WHERE id = ?1", [id]);
        let _ = self.db.execute("DELETE FROM folder_set_items WHERE folder_id = ?1", [id]);
    }

    pub fn folder_sets(&self) -> Vec<FolderSet> {
        let Ok(mut stmt) = self.db.prepare(
            "SELECT s.id, s.name, (SELECT GROUP_CONCAT(i.folder_id) FROM folder_set_items i WHERE i.set_id = s.id)
             FROM folder_sets s ORDER BY s.created_at, s.id",
        ) else {
            return Vec::new();
        };
        stmt.query_map([], |r| {
            let folders: Option<String> = r.get(2)?;
            Ok(FolderSet {
                id: r.get(0)?,
                name: r.get(1)?,
                folders: folders.map(|f| f.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default(),
            })
        })
        .map(|it| it.flatten().collect())
        .unwrap_or_default()
    }

    pub fn create_folder_set(&self, name: &str, folders: &[i64]) -> Option<i64> {
        self.db.execute("INSERT INTO folder_sets (name, created_at) VALUES (?1, ?2)", params![name, now_secs()]).ok()?;
        let id = self.db.last_insert_rowid();
        for folder in folders {
            let _ = self.db.execute("INSERT OR IGNORE INTO folder_set_items VALUES (?1, ?2)", params![id, folder]);
        }
        Some(id)
    }

    pub fn delete_folder_set(&self, id: i64) {
        let _ = self.db.execute("DELETE FROM folder_set_items WHERE set_id = ?1", [id]);
        let _ = self.db.execute("DELETE FROM folder_sets WHERE id = ?1", [id]);
    }

    /// Groups the viewer said aren't duplicates, by `duplicates::Group::key`.
    pub fn ignored_duplicates(&self) -> std::collections::HashSet<String> {
        let Ok(mut stmt) = self.db.prepare("SELECT key FROM dup_ignored") else {
            return Default::default();
        };
        stmt.query_map([], |r| r.get(0)).map(|it| it.flatten().collect()).unwrap_or_default()
    }

    pub fn ignore_duplicate(&self, key: &str, ignored: bool) {
        let sql = if ignored { "INSERT OR IGNORE INTO dup_ignored VALUES (?1)" } else { "DELETE FROM dup_ignored WHERE key = ?1" };
        let _ = self.db.execute(sql, [key]);
    }

    /// Records that a file now lives at `to`. History is keyed by path, so
    /// its progress, plays, bookmarks and VR format move with it.
    pub fn record_move(&self, id: i64, to: &Path, folder_id: i64) {
        let Ok(old) = self.db.query_row("SELECT key FROM videos WHERE id = ?1", [id], |r| r.get::<_, String>(0)) else {
            return;
        };
        let path = display_path(to);
        let size = std::fs::metadata(to).map_or(0, |m| m.len());
        let key = crate::history::file_key(&path).unwrap_or_else(|| format!("{path}|{size}"));
        let title = to.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
        let _ = self.db.execute_batch("BEGIN");
        let _ = self.db.execute(
            "UPDATE videos SET path = ?2, key = ?3, title = ?4, folder_id = ?5 WHERE id = ?1",
            params![id, path, key, title, folder_id],
        );
        for table in ["history", "play_events", "bookmarks"] {
            let verb = if table == "history" { "UPDATE OR REPLACE" } else { "UPDATE" };
            let sql = format!("{verb} {table} SET key = ?1, path = ?2 WHERE key = ?3");
            let _ = self.db.execute(&sql, params![key, path, old]);
        }
        let _ = self.db.execute("UPDATE history SET title = ?1 WHERE key = ?2", params![title, key]);
        let _ = self.db.execute(
            "UPDATE OR REPLACE settings SET k = ?1 WHERE k = ?2",
            params![format!("vr:{key}"), format!("vr:{old}")],
        );
        let _ = self.db.execute_batch("COMMIT");
    }

    pub fn folders(&self) -> Vec<Folder> {
        let Ok(mut stmt) = self.db.prepare(
            "SELECT f.id, f.path, (SELECT COUNT(*) FROM videos v WHERE v.folder_id = f.id AND v.missing = 0),
                    (SELECT COUNT(*) FROM videos v WHERE v.folder_id = f.id AND v.missing != 0)
             FROM folders f ORDER BY f.path",
        ) else {
            return Vec::new();
        };
        stmt.query_map([], |r| Ok(Folder { id: r.get(0)?, path: r.get(1)?, count: r.get(2)?, missing: r.get(3)? }))
            .map(|it| it.flatten().collect())
            .unwrap_or_default()
    }

    // ---- Videos ----

    /// Inserts or refreshes a file found by the scanner. Returns true if the
    /// file is new or changed (and so needs probing).
    pub fn upsert_file(&self, folder_id: i64, path: &Path, size: u64, mtime: i64) -> bool {
        let p = display_path(path);
        // Reads the disk, so only for files that need it.
        let key = || crate::history::file_key(&p).unwrap_or_else(|| format!("{p}|{size}"));
        let existing: Option<(i64, i64, i64)> = self
            .db
            .query_row("SELECT id, size, mtime FROM videos WHERE path = ?1", [&p], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()
            .ok()
            .flatten();
        match existing {
            Some((id, s, m)) if s == size as i64 && m == mtime => {
                let _ = self.db.execute("UPDATE videos SET missing = 0, folder_id = ?2 WHERE id = ?1", params![id, folder_id]);
                false
            }
            Some((id, ..)) => {
                let _ = self.db.execute(
                    "UPDATE videos SET size = ?2, mtime = ?3, key = ?4, probed = 0, missing = 0 WHERE id = ?1",
                    params![id, size as i64, mtime, key()],
                );
                true
            }
            None => {
                let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
                let _ = self.db.execute(
                    "INSERT INTO videos (folder_id, path, key, title, size, mtime, added_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![folder_id, p, key(), title, size as i64, mtime, now_secs()],
                );
                true
            }
        }
    }

    /// Flags every video of `folder_id` not in `seen` as missing.
    pub fn mark_missing(&self, folder_id: i64, seen: &std::collections::HashSet<String>) {
        let Ok(mut stmt) = self.db.prepare("SELECT id, path FROM videos WHERE folder_id = ?1 AND missing = 0") else {
            return;
        };
        let rows: Vec<(i64, String)> = stmt
            .query_map([folder_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|it| it.flatten().collect())
            .unwrap_or_default();
        for (id, path) in rows {
            if !seen.contains(&path) {
                let _ = self.db.execute("UPDATE videos SET missing = 1 WHERE id = ?1", [id]);
            }
        }
    }

    /// Videos still waiting for duration/thumbnail, newest first.
    pub fn pending_probe(&self) -> Vec<(i64, String)> {
        let Ok(mut stmt) = self
            .db
            .prepare("SELECT id, path FROM videos WHERE probed = 0 AND missing = 0 ORDER BY added_at DESC, id DESC")
        else {
            return Vec::new();
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|it| it.flatten().collect())
            .unwrap_or_default()
    }

    pub fn set_probe_result(&self, id: i64, info: Option<&crate::probe::Info>) {
        match info {
            Some(i) => {
                let thumb = i.thumb.as_ref().map(|t| t.to_string_lossy().into_owned());
                let _ = self.db.execute(
                    "UPDATE videos SET duration = ?2, width = ?3, height = ?4, thumb = ?5, probed = 1 WHERE id = ?1",
                    params![id, i.duration, i.width, i.height, thumb],
                );
            }
            None => {
                let _ = self.db.execute("UPDATE videos SET probed = 2 WHERE id = ?1", [id]);
            }
        }
    }

    pub fn remove_video(&self, id: i64) {
        let _ = self.db.execute("DELETE FROM video_tags WHERE video_id = ?1", [id]);
        let _ = self.db.execute("DELETE FROM collection_items WHERE video_id = ?1", [id]);
        let _ = self.db.execute("DELETE FROM videos WHERE id = ?1", [id]);
    }

    /// Every present video with its tags, resume position and play count.
    pub fn videos(&self) -> Vec<Video> {
        let Ok(mut stmt) = self.db.prepare(
            "SELECT v.id, v.path, v.title, v.folder_id, v.duration, v.width, v.height, v.thumb, v.added_at,
                    h.position, h.updated_at,
                    (SELECT COUNT(*) FROM play_events p WHERE p.key = v.key),
                    (SELECT GROUP_CONCAT(t.name, char(31)) FROM video_tags vt JOIN tags t ON t.id = vt.tag_id
                      WHERE vt.video_id = v.id),
                    (SELECT GROUP_CONCAT(ci.collection_id) FROM collection_items ci WHERE ci.video_id = v.id),
                    v.size, v.mtime
             FROM videos v LEFT JOIN history h ON h.key = v.key
             WHERE v.missing = 0 AND v.folder_id IN (SELECT id FROM folders)",
        ) else {
            return Vec::new();
        };
        stmt.query_map([], |r| {
            let tags: Option<String> = r.get(12)?;
            Ok(Video {
                id: r.get(0)?,
                path: r.get(1)?,
                title: r.get(2)?,
                folder_id: r.get(3)?,
                duration: r.get(4)?,
                width: r.get(5)?,
                height: r.get(6)?,
                thumb: r.get(7)?,
                added_at: r.get(8)?,
                position: r.get(9)?,
                last_played: r.get(10)?,
                plays: r.get(11)?,
                tags: tags
                    .map(|t| t.split('\u{1f}').map(str::to_string).collect())
                    .unwrap_or_default(),
                collections: r
                    .get::<_, Option<String>>(13)?
                    .map(|c| c.split(',').filter_map(|x| x.parse().ok()).collect())
                    .unwrap_or_default(),
                size: r.get(14)?,
                mtime: r.get(15)?,
            })
        })
        .map(|it| it.flatten().collect())
        .unwrap_or_default()
    }

    /// Library id for a file path, if the file is in the library.
    pub fn video_id_for_path(&self, path: &str) -> Option<i64> {
        let canon = std::fs::canonicalize(path).ok().map(|p| display_path(&p));
        let p = canon.as_deref().unwrap_or(path);
        self.db.query_row("SELECT id FROM videos WHERE path = ?1 AND missing = 0", [p], |r| r.get(0)).ok()
    }

    // ---- Collections (favorites) ----

    /// All collections; creates "默认收藏" the first time so there's always one.
    pub fn collections(&self) -> Vec<Collection> {
        let n: i64 = self.db.query_row("SELECT COUNT(*) FROM collections", [], |r| r.get(0)).unwrap_or(0);
        if n == 0 {
            self.create_collection("默认收藏");
        }
        let Ok(mut stmt) = self.db.prepare(
            "SELECT c.id, c.name,
                    (SELECT COUNT(*) FROM collection_items ci JOIN videos v ON v.id = ci.video_id
                      WHERE ci.collection_id = c.id AND v.missing = 0)
             FROM collections c ORDER BY c.sort_order, c.id",
        ) else {
            return Vec::new();
        };
        stmt.query_map([], |r| Ok(Collection { id: r.get(0)?, name: r.get(1)?, count: r.get(2)? }))
            .map(|it| it.flatten().collect())
            .unwrap_or_default()
    }

    pub fn create_collection(&self, name: &str) -> Option<i64> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        self.db
            .execute(
                "INSERT INTO collections (name, sort_order, created_at)
                 VALUES (?1, (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM collections), ?2)",
                params![name, now_secs()],
            )
            .ok()?;
        Some(self.db.last_insert_rowid())
    }

    /// Deletes a collection (the videos stay in the library).
    pub fn delete_collection(&self, id: i64) {
        let _ = self.db.execute("DELETE FROM collection_items WHERE collection_id = ?1", [id]);
        let _ = self.db.execute("DELETE FROM collections WHERE id = ?1", [id]);
    }

    pub fn set_in_collection(&self, collection_id: i64, video_id: i64, on: bool) {
        if on {
            let _ = self.db.execute(
                "INSERT OR IGNORE INTO collection_items (collection_id, video_id, added_at) VALUES (?1, ?2, ?3)",
                params![collection_id, video_id, now_secs()],
            );
        } else {
            let _ = self.db.execute(
                "DELETE FROM collection_items WHERE collection_id = ?1 AND video_id = ?2",
                params![collection_id, video_id],
            );
        }
    }

    pub fn is_favorite(&self, video_id: i64) -> bool {
        self.db
            .query_row("SELECT 1 FROM collection_items WHERE video_id = ?1 LIMIT 1", [video_id], |_| Ok(()))
            .is_ok()
    }

    /// Quick favorite (player star / Ctrl+D): adds to the first collection,
    /// or removes from every collection if already favorited.
    /// Returns the new state.
    pub fn toggle_favorite(&self, video_id: i64) -> bool {
        if self.is_favorite(video_id) {
            let _ = self.db.execute("DELETE FROM collection_items WHERE video_id = ?1", [video_id]);
            false
        } else if let Some(first) = self.collections().first() {
            self.set_in_collection(first.id, video_id, true);
            true
        } else {
            false
        }
    }

    // ---- Tags ----

    pub fn add_tag(&self, video_id: i64, name: &str) {
        let name = name.trim().trim_start_matches('#').trim();
        if name.is_empty() {
            return;
        }
        let _ = self.db.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [name]);
        let _ = self.db.execute(
            "INSERT OR IGNORE INTO video_tags (video_id, tag_id) SELECT ?1, id FROM tags WHERE name = ?2",
            params![video_id, name],
        );
    }

    pub fn remove_tag(&self, video_id: i64, name: &str) {
        let _ = self.db.execute(
            "DELETE FROM video_tags WHERE video_id = ?1 AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
            params![video_id, name],
        );
        // Drop tags nobody uses any more.
        let _ = self.db.execute("DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM video_tags)", []);
    }
}

/// Paths are stored without Windows' `\\?\` prefix so they match what the
/// rest of the app (and mpv) uses.
pub fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_a_file_keeps_its_records() {
        let dir = std::env::temp_dir().join(format!("tinyplayer-lib-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        let db = dir.join("t.db");
        let store = crate::history::Store::open(&db).unwrap();
        let lib = Library::open(&db).unwrap();

        let from = dir.join("a/film.mp4");
        std::fs::write(&from, b"12345").unwrap();
        let folder = lib.add_folder(dir.to_str().unwrap()).unwrap();
        lib.upsert_file(folder, &from, 5, 1);
        let old = crate::history::file_key(from.to_str().unwrap()).unwrap();
        let id = lib.videos()[0].id;
        store.save_position(&old, from.to_str().unwrap(), "film", 12.0, 60.0);
        store.record_play(&old, from.to_str().unwrap(), 1);
        store.add_bookmark(&old, from.to_str().unwrap(), 3.0, "x");
        store.set_value(&format!("vr:{old}"), "360");
        lib.add_tag(id, "keep");

        let to = dir.join("b/film (2).mp4");
        std::fs::rename(&from, &to).unwrap();
        lib.record_move(id, &to, folder);

        let new = crate::history::file_key(to.to_str().unwrap()).unwrap();
        let v = &lib.videos()[0];
        assert_eq!(v.id, id);
        assert_eq!(v.title, "film (2)");
        assert_eq!(v.position, Some(12.0));
        assert_eq!(store.recent(1)[0].title, "film (2)");
        assert_eq!(v.plays, 1);
        assert_eq!(v.tags, ["keep"]);
        assert_eq!(store.bookmarks(&new).len(), 1);
        assert_eq!(store.get_value(&format!("vr:{new}")).as_deref(), Some("360"));
        assert_eq!(store.get_value(&format!("vr:{old}")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clearing_forgets_the_library_but_not_the_files() {
        let dir = std::env::temp_dir().join(format!("tinyplayer-lib-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lib = Library::open(&dir.join("t.db")).unwrap();
        let file = dir.join("a.mp4");
        std::fs::write(&file, b"x").unwrap();
        let folder = lib.add_folder(dir.to_str().unwrap()).unwrap();
        lib.upsert_file(folder, &file, 1, 1);
        lib.add_tag(lib.videos()[0].id, "t");
        lib.create_folder_set("s", &[folder]);
        lib.clear();
        assert!(lib.videos().is_empty());
        assert!(lib.folders().is_empty());
        assert!(lib.folder_sets().is_empty());
        assert!(file.exists());

        // A scan still running when the library was cleared.
        lib.upsert_file(folder, &file, 1, 1);
        assert!(lib.videos().is_empty());
        lib.remove_orphans();
        lib.upsert_file(99, &file, 1, 1);
        lib.remove_orphans();
        assert!(lib.pending_probe().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_sets_and_ignored_groups_are_kept() {
        let dir = std::env::temp_dir().join(format!("tinyplayer-lib-sets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lib = Library::open(&dir.join("t.db")).unwrap();
        let folder = lib.add_folder(dir.to_str().unwrap()).unwrap();
        let set = lib.create_folder_set("影视", &[folder]).unwrap();
        assert_eq!(lib.folder_sets()[0].folders, [folder]);
        lib.remove_folder(folder);
        assert!(lib.folder_sets()[0].folders.is_empty());
        lib.delete_folder_set(set);
        assert!(lib.folder_sets().is_empty());

        lib.ignore_duplicate("1-2", true);
        assert!(lib.ignored_duplicates().contains("1-2"));
        lib.ignore_duplicate("1-2", false);
        assert!(lib.ignored_duplicates().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_lib() -> (Library, PathBuf) {
        let dir = std::env::temp_dir().join(format!("tp-lib-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("media")).unwrap();
        let lib = Library::open(&dir.join("t.db")).unwrap();
        (lib, dir)
    }

    #[test]
    fn scan_tag_and_remove() {
        let (lib, dir) = temp_lib();
        let media = dir.join("media");
        let file = media.join("第1讲.mp4");
        std::fs::write(&file, b"x").unwrap();
        let fid = lib.add_folder(&media.to_string_lossy()).unwrap();
        assert_eq!(lib.add_folder(&media.to_string_lossy()), Some(fid), "adding twice is a no-op");

        let canon = std::fs::canonicalize(&file).unwrap();
        assert!(lib.upsert_file(fid, &canon, 1, 100), "new file needs probing");
        assert!(!lib.upsert_file(fid, &canon, 1, 100), "unchanged file is skipped");
        assert!(lib.upsert_file(fid, &canon, 2, 200), "changed file is re-probed");

        let v = &lib.videos()[0];
        assert_eq!(v.title, "第1讲");
        lib.add_tag(v.id, "#课程");
        lib.add_tag(v.id, "Rust");
        let mut tags = lib.videos()[0].tags.clone();
        tags.sort();
        assert_eq!(tags, ["Rust", "课程"]);
        lib.remove_tag(v.id, "Rust");
        assert_eq!(lib.videos()[0].tags, ["课程"]);

        // Collections: a default one exists, quick-favorite toggles it.
        let cols = lib.collections();
        assert_eq!(cols[0].name, "默认收藏");
        assert!(lib.toggle_favorite(v.id));
        assert_eq!(lib.videos()[0].collections, [cols[0].id]);
        let watch = lib.create_collection("想看").unwrap();
        lib.set_in_collection(watch, v.id, true);
        assert_eq!(lib.collections().iter().map(|c| c.count).collect::<Vec<_>>(), [1, 1]);
        assert!(!lib.toggle_favorite(v.id), "toggling again removes from all");
        assert!(lib.videos()[0].collections.is_empty());
        assert_eq!(lib.video_id_for_path(&canon.to_string_lossy()), Some(v.id));

        lib.mark_missing(fid, &Default::default());
        assert!(lib.videos().is_empty(), "missing files are hidden");

        lib.remove_folder(fid);
        assert!(lib.folders().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
