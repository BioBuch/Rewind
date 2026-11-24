use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::player::extract_mp4_thumbnail_jpg;
use crate::remuxer::probe_mp4_metadata;
use crate::win_ffi::unix_now_secs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipRecord {
    pub id: i64,
    pub file_path: String,
    pub thumb_path: String,
    pub title: String,
    pub game: String,
    pub duration_secs: f32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: String,
    pub size_bytes: u64,
    pub created_at_unix: i64,
    pub favorite: bool,
    pub collection: Option<String>,
}

impl ClipRecord {
    pub fn resolution_badge(&self) -> String {
        if self.height >= 2160 || self.width >= 3840 {
            "4K".to_owned()
        } else if self.height >= 1440 {
            "1440p".to_owned()
        } else if self.height >= 1080 {
            "1080p".to_owned()
        } else if self.height > 0 {
            format!("{}p", self.height)
        } else {
            "Video".to_owned()
        }
    }

    pub fn duration_str(&self) -> String {
        let total = self.duration_secs.round().max(0.0) as u32;
        format!("{}:{:02}", total / 60, total % 60)
    }

    pub fn relative_time(&self, now_unix: i64) -> String {
        let delta = (now_unix - self.created_at_unix).max(0);
        if delta < 60 {
            "Just now".to_owned()
        } else if delta < 3600 {
            format!("{}m ago", delta / 60)
        } else if delta < 86_400 {
            format!("{}h ago", delta / 3600)
        } else {
            format!("{}d ago", delta / 86_400)
        }
    }
}

pub struct ClipLibrary {
    conn: Connection,
}

impl ClipLibrary {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create db dir: {e}"))?;
        }
        let conn = Connection::open(db_path).map_err(|e| format!("open sqlite: {e}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS clips (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL UNIQUE,
                thumb_path TEXT NOT NULL DEFAULT '',
                title TEXT NOT NULL,
                game TEXT NOT NULL,
                duration_secs REAL NOT NULL,
                width INTEGER NOT NULL,
                height INTEGER NOT NULL,
                fps INTEGER NOT NULL,
                codec TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                created_at_unix INTEGER NOT NULL,
                favorite INTEGER NOT NULL DEFAULT 0,
                collection TEXT,
                deleted_at_unix INTEGER
            );",
        )
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self { conn })
    }

    pub fn register_clip(
        &self,
        file_path: &Path,
        clips_root: &Path,
        title: &str,
        game: &str,
        duration_secs: f32,
        width: u32,
        height: u32,
        fps: u32,
        codec: &str,
        created_at_unix: i64,
    ) -> Result<ClipRecord, String> {
        let size_bytes = fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);
        let thumb_path = thumb_path_for(clips_root, file_path);
        if let Some(parent) = thumb_path.parent() {
            let _ = fs::create_dir_all(parent);
            crate::win_ffi::hide_path(parent);
        }
        if !thumb_path.exists() {
            let seek = (duration_secs * 0.15).min(0.5);
            let _ = extract_mp4_thumbnail_jpg(file_path, &thumb_path, seek);
        }
        let fp_str = file_path.to_string_lossy().into_owned();
        let tp_str = if thumb_path.exists() {
            thumb_path.to_string_lossy().into_owned()
        } else {
            String::new()
        };

        self.conn
            .execute(
                "INSERT INTO clips (
                    file_path, thumb_path, title, game, duration_secs, width, height, fps, codec,
                    size_bytes, created_at_unix, favorite, collection, deleted_at_unix
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, NULL, NULL)
                ON CONFLICT(file_path) DO UPDATE SET
                    thumb_path = excluded.thumb_path,
                    duration_secs = excluded.duration_secs,
                    width = excluded.width,
                    height = excluded.height,
                    fps = excluded.fps,
                    codec = excluded.codec,
                    size_bytes = excluded.size_bytes",
                params![
                    fp_str,
                    tp_str,
                    title,
                    game,
                    duration_secs,
                    width,
                    height,
                    fps,
                    codec,
                    size_bytes as i64,
                    created_at_unix,
                ],
            )
            .map_err(|e| format!("insert clip: {e}"))?;

        self.get_by_path(&fp_str)?
            .ok_or_else(|| "Failed to retrieve inserted clip".to_owned())
    }

    pub fn sync_folder(&self, clips_root: &Path) -> Result<(), String> {
        let _ = fs::create_dir_all(clips_root);
        let thumb_dir = clips_root.join(".thumbnails");
        if thumb_dir.exists() {
            crate::win_ffi::hide_path(&thumb_dir);
        }
        let existing = self.list_all_including_deleted()?;
        for rec in &existing {
            if !Path::new(&rec.file_path).exists() {
                if !rec.thumb_path.is_empty() {
                    let _ = fs::remove_file(&rec.thumb_path);
                }
                let _ = self
                    .conn
                    .execute("DELETE FROM clips WHERE id = ?1", params![rec.id]);
            } else if rec.thumb_path.is_empty() || !Path::new(&rec.thumb_path).exists() {
                let tp = thumb_path_for(clips_root, Path::new(&rec.file_path));
                if extract_mp4_thumbnail_jpg(Path::new(&rec.file_path), &tp, 0.2).is_ok() {
                    let _ = self.conn.execute(
                        "UPDATE clips SET thumb_path = ?1 WHERE id = ?2",
                        params![tp.to_string_lossy().into_owned(), rec.id],
                    );
                }
            }
        }

        let disk_files = scan_media_files(clips_root);
        for file in disk_files {
            let fp_str = file.to_string_lossy().into_owned();
            if self.get_by_path(&fp_str)?.is_some() {
                continue;
            }
            let meta = probe_mp4_metadata(&file).ok();
            let title = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Captured Clip")
                .to_owned();
            let game = infer_game_from_path(&file, clips_root);
            let created = fs::metadata(&file)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or_else(unix_now_secs);

            let (dur, w, h, fps, codec) = match meta {
                Some(m) => (m.duration_secs, m.width, m.height, m.fps, m.codec),
                None => (0.0, 1920, 1080, 60, "MP4".to_owned()),
            };
            let _ = self.register_clip(&file, clips_root, &title, &game, dur, w, h, fps, &codec, created);
        }
        self.prune_orphan_thumbnails(clips_root)?;
        Ok(())
    }

    fn prune_orphan_thumbnails(&self, clips_root: &Path) -> Result<(), String> {
        let thumb_dir = clips_root.join(".thumbnails");
        if !thumb_dir.is_dir() {
            return Ok(());
        }
        let mut live_thumbs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for rec in self.list_all_including_deleted()? {
            if !rec.thumb_path.is_empty() {
                live_thumbs.insert(PathBuf::from(&rec.thumb_path));
            }
            live_thumbs.insert(thumb_path_for(clips_root, Path::new(&rec.file_path)));
        }
        if let Ok(entries) = fs::read_dir(&thumb_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && !live_thumbs.contains(&p) {
                    let _ = fs::remove_file(p);
                }
            }
        }
        Ok(())
    }

    pub fn list_active_clips(&self) -> Result<Vec<ClipRecord>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, thumb_path, title, game, duration_secs, width, height, fps, codec,
                        size_bytes, created_at_unix, favorite, collection
                 FROM clips
                 WHERE deleted_at_unix IS NULL
                 ORDER BY created_at_unix DESC, id DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_clip)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            if let Ok(rec) = r {
                out.push(rec);
            }
        }
        Ok(out)
    }

    fn list_all_including_deleted(&self) -> Result<Vec<ClipRecord>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, thumb_path, title, game, duration_secs, width, height, fps, codec,
                        size_bytes, created_at_unix, favorite, collection
                 FROM clips",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], row_to_clip).map_err(|e| e.to_string())?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn get_by_id(&self, id: i64) -> Result<Option<ClipRecord>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, thumb_path, title, game, duration_secs, width, height, fps, codec,
                        size_bytes, created_at_unix, favorite, collection
                 FROM clips WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query_map(params![id], row_to_clip).map_err(|e| e.to_string())?;
        Ok(rows.next().and_then(Result::ok))
    }

    fn get_by_path(&self, fp: &str) -> Result<Option<ClipRecord>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, thumb_path, title, game, duration_secs, width, height, fps, codec,
                        size_bytes, created_at_unix, favorite, collection
                 FROM clips WHERE file_path = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query_map(params![fp], row_to_clip).map_err(|e| e.to_string())?;
        Ok(rows.next().and_then(Result::ok))
    }

    pub fn toggle_favorite(&self, id: i64) -> Result<bool, String> {
        self.conn
            .execute(
                "UPDATE clips SET favorite = CASE WHEN favorite = 0 THEN 1 ELSE 0 END WHERE id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
        let rec = self.get_by_id(id)?.ok_or_else(|| "Clip not found".to_owned())?;
        Ok(rec.favorite)
    }

    pub fn rename_clip(&self, id: i64, new_title: &str) -> Result<ClipRecord, String> {
        let clean_title = new_title.trim();
        if clean_title.is_empty() {
            return Err("Title cannot be empty".to_owned());
        }
        let rec = self.get_by_id(id)?.ok_or_else(|| "Clip not found".to_owned())?;
        let old_path = Path::new(&rec.file_path);
        let ext = old_path.extension().and_then(|s| s.to_str()).unwrap_or("mp4");
        let safe_stem = sanitize_filename(clean_title);
        let mut new_path = old_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{safe_stem}.{ext}"));

        if new_path != old_path {
            let same_ci = old_path
                .to_string_lossy()
                .eq_ignore_ascii_case(&new_path.to_string_lossy());
            if !same_ci && new_path.exists() {
                new_path = old_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(format!("{safe_stem}_{id}.{ext}"));
            }
            if old_path.exists() {
                fs::rename(old_path, &new_path).map_err(|e| format!("rename file: {e}"))?;
            }
        }

        let new_fp_str = new_path.to_string_lossy().into_owned();
        self.conn
            .execute(
                "UPDATE clips SET title = ?1, file_path = ?2 WHERE id = ?3",
                params![clean_title, new_fp_str, id],
            )
            .map_err(|e| e.to_string())?;
        self.get_by_id(id)?.ok_or_else(|| "Clip missing after rename".to_owned())
    }

    pub fn set_collection(&self, id: i64, collection: Option<&str>) -> Result<(), String> {
        let col_val = collection.map(str::trim).filter(|s| !s.is_empty());
        self.conn
            .execute(
                "UPDATE clips SET collection = ?1 WHERE id = ?2",
                params![col_val, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn soft_delete_clip(&self, id: i64, now_unix: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE clips SET deleted_at_unix = ?1 WHERE id = ?2",
                params![now_unix, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn undo_delete_clip(&self, id: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE clips SET deleted_at_unix = NULL WHERE id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn purge_expired_deletes(&self, now_unix: i64, grace_secs: i64) -> Result<usize, String> {
        let cutoff = now_unix - grace_secs;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, thumb_path FROM clips WHERE deleted_at_unix IS NOT NULL AND deleted_at_unix <= ?1",
            )
            .map_err(|e| e.to_string())?;
        let targets: Vec<(i64, String, String)> = stmt
            .query_map(params![cutoff], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();

        let count = targets.len();
        for (id, fp, tp) in targets {
            let _ = fs::remove_file(&fp);
            if !tp.is_empty() {
                let _ = fs::remove_file(&tp);
            }
            let _ = self.conn.execute("DELETE FROM clips WHERE id = ?1", params![id]);
        }
        Ok(count)
    }

    pub fn enforce_quota(
        &self,
        clips_root: &Path,
        max_storage_bytes: u64,
        auto_delete_oldest: bool,
    ) -> Result<usize, String> {
        if !auto_delete_oldest || max_storage_bytes == 0 {
            return Ok(0);
        }
        let mut removed = 0;
        loop {
            let used = total_disk_usage_bytes(clips_root);
            if used <= max_storage_bytes {
                break;
            }
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, file_path, thumb_path FROM clips
                     WHERE favorite = 0 AND deleted_at_unix IS NULL
                     ORDER BY created_at_unix ASC, id ASC LIMIT 1",
                )
                .map_err(|e| e.to_string())?;
            let oldest = stmt
                .query_row([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .ok();
            let Some((id, fp, tp)) = oldest else {
                break;
            };
            let _ = fs::remove_file(&fp);
            if !tp.is_empty() {
                let _ = fs::remove_file(&tp);
            }
            let _ = self.conn.execute("DELETE FROM clips WHERE id = ?1", params![id]);
            removed += 1;
        }
        Ok(removed)
    }
}

fn row_to_clip(r: &rusqlite::Row<'_>) -> rusqlite::Result<ClipRecord> {
    Ok(ClipRecord {
        id: r.get(0)?,
        file_path: r.get(1)?,
        thumb_path: r.get(2)?,
        title: r.get(3)?,
        game: r.get(4)?,
        duration_secs: r.get::<_, f64>(5)? as f32,
        width: r.get::<_, i64>(6)? as u32,
        height: r.get::<_, i64>(7)? as u32,
        fps: r.get::<_, i64>(8)? as u32,
        codec: r.get(9)?,
        size_bytes: r.get::<_, i64>(10)? as u64,
        created_at_unix: r.get(11)?,
        favorite: r.get::<_, i64>(12)? != 0,
        collection: r.get(13)?,
    })
}

pub fn total_disk_usage_bytes(root: &Path) -> u64 {
    let mut sum = 0_u64;
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name != ".thumbnails" {
                    sum += total_disk_usage_bytes(&p);
                }
            } else if let Ok(meta) = entry.metadata() {
                sum += meta.len();
            }
        }
    }
    sum
}

fn scan_media_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if !name.starts_with('.') {
                    out.extend(scan_media_files(&p));
                }
            } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                let lower = ext.to_ascii_lowercase();
                if lower == "mp4" || lower == "mkv" {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn infer_game_from_path(file: &Path, root: &Path) -> String {
    if let Ok(rel) = file.strip_prefix(root) {
        let mut comps = rel.components();
        if let Some(first) = comps.next() {
            if comps.next().is_some() {
                return first.as_os_str().to_string_lossy().into_owned();
            }
        }
    }
    "Desktop".to_owned()
}

pub fn thumb_path_for(clips_root: &Path, video_path: &Path) -> PathBuf {
    let stem = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip");
    clips_root.join(".thumbnails").join(format!("{stem}.jpg"))
}

pub fn sanitize_filename(name: &str) -> String {
    let out: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            _ => c,
        })
        .collect();
    let trimmed = out.trim().trim_matches('.');
    if trimmed.is_empty() {
        return "Rewind_Clip".to_owned();
    }
    let stem_upper = trimmed
        .split('.')
        .next()
        .unwrap_or(trimmed)
        .trim()
        .to_ascii_uppercase();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem_upper.as_str()) {
        format!("{trimmed}_clip")
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_library_rename_soft_delete_undo_and_quota_prune() {
        let dir = std::env::temp_dir().join("rewind_lib_test");
        let db_dir = std::env::temp_dir().join("rewind_lib_test_db");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&db_dir);
        fs::create_dir_all(&dir).unwrap();
        let db = ClipLibrary::open(&db_dir.join("clips.db")).unwrap();

        let f1 = dir.join("clip1.mp4");
        let f2 = dir.join("clip2.mp4");
        fs::write(&f1, vec![0_u8; 4096]).unwrap();
        fs::write(&f2, vec![0_u8; 4096]).unwrap();

        let _r1 = db
            .register_clip(&f1, &dir, "Old Clip", "Valorant", 30.0, 1920, 1080, 60, "HEVC", 100)
            .unwrap();
        let r2 = db
            .register_clip(&f2, &dir, "Fav Clip", "CS2", 15.0, 1920, 1080, 60, "HEVC", 200)
            .unwrap();

        assert!(db.toggle_favorite(r2.id).unwrap());
        assert_eq!(total_disk_usage_bytes(&dir), 8192);

        let pruned = db.enforce_quota(&dir, 5000, true).unwrap();
        assert_eq!(pruned, 1);
        assert!(!f1.exists());
        assert!(f2.exists());

        let renamed = db.rename_clip(r2.id, "Renamed Clutch").unwrap();
        assert_eq!(renamed.title, "Renamed Clutch");
        assert!(Path::new(&renamed.file_path).exists());

        let case_renamed = db.rename_clip(r2.id, "renamed clutch").unwrap();
        assert_eq!(case_renamed.title, "renamed clutch");
        assert!(case_renamed.file_path.ends_with("renamed clutch.mp4"));
        assert!(Path::new(&case_renamed.file_path).exists());
        assert_eq!(sanitize_filename("CON"), "CON_clip");
        assert_eq!(sanitize_filename("aux.mp4"), "aux.mp4_clip");

        db.soft_delete_clip(r2.id, 1000).unwrap();
        assert!(db.list_active_clips().unwrap().is_empty());
        db.undo_delete_clip(r2.id).unwrap();
        assert_eq!(db.list_active_clips().unwrap().len(), 1);

        drop(db);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&db_dir);
    }
}
