use anyhow::{Context, Result};
use rusqlite::{ffi::sqlite3_auto_extension, params, Connection, OpenFlags, OptionalExtension};
use sqlite_vec::sqlite3_vec_init;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use zerocopy::IntoBytes;

use crate::parser::ParsedQuery;

pub const EMBED_DIM: usize = 256;

#[derive(Debug, Clone, Default)]
pub struct ExifRecord {
    pub captured_at: Option<i64>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_model: Option<String>,
    pub focal_length_mm: Option<f64>,
    pub aperture_f: Option<f64>,
    pub iso: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub blake3_hash: String,
    pub rel_path: String,
    pub distance: f64,
    pub focal_length_mm: Option<f64>,
    pub aperture_f: Option<f64>,
    pub camera_model: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CatalogStats {
    pub assets: i64,
    pub paths: i64,
    pub model: Option<String>,
}

pub struct CatalogDb {
    pub conn: Connection,
}

fn install_vec() {
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
    }
}

/// `file:/abs/path?immutable=1`, with bytes outside the URI-safe set percent-encoded.
fn immutable_uri(db_path: &Path) -> String {
    let mut uri = String::from("file:");
    for byte in db_path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(byte as char);
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri.push_str("?immutable=1");
    uri
}

impl CatalogDb {
    pub fn open(db_path: &Path) -> Result<Self> {
        install_vec();

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open catalog DB at {:?}", db_path))?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA mmap_size = 268435456;
             PRAGMA temp_store = MEMORY;
             PRAGMA foreign_keys = ON;",
        )?;

        let db = Self { conn };
        db.init_schema()?;
        db.wal_checkpoint();
        Ok(db)
    }

    /// Open a catalog that already exists. Lookup never creates a database on the disk it is reading.
    pub fn open_readonly(db_path: &Path) -> Result<Self> {
        if !db_path.is_file() {
            anyhow::bail!("no photo index at {}", db_path.display());
        }
        install_vec();

        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
        if let Ok(conn) = Connection::open_with_flags(db_path, flags) {
            let db = Self { conn };
            if db.has_vec_media() {
                return Ok(db);
            }
        }

        let uri = immutable_uri(db_path);
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(&uri, flags)
            .with_context(|| format!("Failed to read catalog at {}", db_path.display()))?;
        let db = Self { conn };
        if !db.has_vec_media() {
            anyhow::bail!("index is still updating");
        }
        Ok(db)
    }

    pub fn wal_checkpoint(&self) {
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }

    fn has_vec_media(&self) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE name = 'vec_media' LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .ok()
            .is_some()
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS media_assets (
                blake3_hash     TEXT PRIMARY KEY,
                file_size       INTEGER NOT NULL,
                mime_type       TEXT NOT NULL,
                indexed_at      INTEGER NOT NULL,
                model_version   TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS file_paths (
                path_id         INTEGER PRIMARY KEY AUTOINCREMENT,
                blake3_hash     TEXT NOT NULL REFERENCES media_assets(blake3_hash) ON DELETE CASCADE,
                rel_path        TEXT NOT NULL UNIQUE,
                file_mtime      INTEGER NOT NULL,
                file_size       INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_rel_path ON file_paths(rel_path);

            CREATE TABLE IF NOT EXISTS exif_metadata (
                blake3_hash     TEXT PRIMARY KEY REFERENCES media_assets(blake3_hash) ON DELETE CASCADE,
                captured_at     INTEGER,
                camera_make     TEXT,
                camera_model    TEXT,
                lens_model      TEXT,
                focal_length_mm REAL,
                aperture_f      REAL,
                iso             INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_exif_filter
                ON exif_metadata(captured_at, focal_length_mm, aperture_f, iso);

            CREATE VIRTUAL TABLE IF NOT EXISTS vec_media USING vec0(
                blake3_hash TEXT PRIMARY KEY,
                embedding float[{dim}] distance_metric=cosine
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS media_fts USING fts5(
                blake3_hash UNINDEXED,
                body
            );",
            dim = EMBED_DIM
        ))?;
        Ok(())
    }

    pub fn stats(&self) -> Result<CatalogStats> {
        let assets: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM media_assets", [], |row| row.get(0))?;
        let paths: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM file_paths", [], |row| row.get(0))?;
        let model: Option<String> = self
            .conn
            .query_row(
                "SELECT model_version FROM media_assets LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(CatalogStats {
            assets,
            paths,
            model,
        })
    }

    /// Drop every indexed row. Used on a fresh rebuild database, never on the
    /// live catalog while search still needs the previous model.
    #[allow(dead_code)]
    pub fn clear_media(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "DROP TABLE IF EXISTS vec_media;
             DROP TABLE IF EXISTS media_fts;
             DELETE FROM exif_metadata;
             DELETE FROM file_paths;
             DELETE FROM media_assets;",
        )?;
        self.init_schema()?;
        self.wal_checkpoint();
        Ok(())
    }

    /// `catalog.db` plus the WAL/SHM files SQLite keeps next to it.
    fn sqlite_suffix(path: &Path, suffix: &str) -> PathBuf {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(suffix);
        path.with_file_name(name)
    }

    pub fn remove_sqlite_files(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(Self::sqlite_suffix(path, "-wal"));
        let _ = std::fs::remove_file(Self::sqlite_suffix(path, "-shm"));
    }

    /// Put `rebuild` at `live` after a model swap. The previous catalog is
    /// deleted only after the new file is in place. A failed rename puts the
    /// previous catalog back.
    pub fn swap_sqlite(live: &Path, rebuild: &Path) -> Result<()> {
        let legacy = live.with_file_name("catalog.legacy.db");
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(Self::sqlite_suffix(live, suffix));
            let _ = std::fs::remove_file(Self::sqlite_suffix(rebuild, suffix));
            let _ = std::fs::remove_file(Self::sqlite_suffix(&legacy, suffix));
        }
        let _ = std::fs::remove_file(&legacy);
        std::fs::rename(live, &legacy)
            .with_context(|| format!("could not set aside {}", live.display()))?;
        match std::fs::rename(rebuild, live) {
            Ok(()) => {
                let _ = std::fs::remove_file(&legacy);
                Ok(())
            }
            Err(err) => {
                let _ = std::fs::rename(&legacy, live);
                Err(err).with_context(|| {
                    format!("could not install the new catalog at {}", live.display())
                })
            }
        }
    }

    pub fn model_mismatch(&self, current: &str) -> Result<bool> {
        let other: Option<String> = self
            .conn
            .query_row(
                "SELECT model_version FROM media_assets WHERE model_version != ?1 LIMIT 1",
                params![current],
                |row| row.get(0),
            )
            .optional()?;
        Ok(other.is_some())
    }

    /// Drop path rows under `prefix` that this scan did not see, then drop assets
    /// those paths were the last name for. `prefix` is empty when the scan covers
    /// the whole drive root.
    pub fn prune_missing(&mut self, prefix: &str, seen: &HashSet<String>) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut stmt = tx.prepare("SELECT rel_path FROM file_paths")?;
        let existing: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut removed = 0usize;
        for path in existing {
            let in_scope = prefix.is_empty()
                || path == prefix
                || path.starts_with(&format!("{prefix}/"));
            if in_scope && !seen.contains(&path) {
                tx.execute("DELETE FROM file_paths WHERE rel_path = ?1", params![path])?;
                removed += 1;
            }
        }

        let mut stmt = tx.prepare(
            "SELECT blake3_hash FROM media_assets
             WHERE blake3_hash NOT IN (SELECT blake3_hash FROM file_paths)",
        )?;
        let orphans: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        for hash in orphans {
            tx.execute("DELETE FROM vec_media WHERE blake3_hash = ?1", params![hash])?;
            tx.execute("DELETE FROM media_fts WHERE blake3_hash = ?1", params![hash])?;
            tx.execute(
                "DELETE FROM media_assets WHERE blake3_hash = ?1",
                params![hash],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }

    pub fn is_path_unchanged(&self, rel_path: &str, mtime: i64, size: u64) -> Result<bool> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM file_paths WHERE rel_path = ?1 AND file_mtime = ?2 AND file_size = ?3",
                params![rel_path, mtime, size as i64],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn has_hash(&self, blake3_hash: &str) -> Result<bool> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM media_assets WHERE blake3_hash = ?1",
                params![blake3_hash],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn bind_path_to_hash(&self, blake3_hash: &str, rel_path: &str, mtime: i64, size: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO file_paths (blake3_hash, rel_path, file_mtime, file_size)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(rel_path) DO UPDATE SET
                blake3_hash = excluded.blake3_hash,
                file_mtime  = excluded.file_mtime,
                file_size   = excluded.file_size",
            params![blake3_hash, rel_path, mtime, size as i64],
        )?;
        write_fts(&self.conn, blake3_hash, None)?;
        Ok(())
    }

    pub fn insert_full_asset(
        &mut self,
        blake3_hash: &str,
        rel_path: &str,
        mtime: i64,
        size: u64,
        mime_type: &str,
        model_version: &str,
        exif: &ExifRecord,
        embedding: &[f32; EMBED_DIM],
        fts_body: &str,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let now = chrono::Utc::now().timestamp();

        tx.execute(
            "INSERT INTO media_assets (blake3_hash, file_size, mime_type, indexed_at, model_version)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(blake3_hash) DO UPDATE SET
                file_size = excluded.file_size,
                mime_type = excluded.mime_type,
                model_version = excluded.model_version",
            params![blake3_hash, size as i64, mime_type, now, model_version],
        )?;

        tx.execute(
            "INSERT INTO file_paths (blake3_hash, rel_path, file_mtime, file_size)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(rel_path) DO UPDATE SET
                blake3_hash = excluded.blake3_hash,
                file_mtime  = excluded.file_mtime,
                file_size   = excluded.file_size",
            params![blake3_hash, rel_path, mtime, size as i64],
        )?;

        tx.execute(
            "INSERT OR REPLACE INTO exif_metadata
             (blake3_hash, captured_at, camera_make, camera_model, lens_model, focal_length_mm, aperture_f, iso)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                blake3_hash,
                exif.captured_at,
                exif.camera_make,
                exif.camera_model,
                exif.lens_model,
                exif.focal_length_mm,
                exif.aperture_f,
                exif.iso,
            ],
        )?;

        tx.execute(
            "INSERT OR REPLACE INTO vec_media (blake3_hash, embedding) VALUES (?1, ?2)",
            params![blake3_hash, embedding.as_bytes()],
        )?;
        write_fts(&tx, blake3_hash, Some(fts_body))?;

        tx.commit()?;
        Ok(())
    }

    pub fn search_hybrid(
        &self,
        query_vec: &[f32; EMBED_DIM],
        parsed: &ParsedQuery,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let mut hits = if parsed.has_exif_filter() {
            self.score_filtered(query_vec, parsed, limit.max(50))?
        } else {
            self.search_knn(query_vec, limit)?
        };
        if let Ok(words) = self.search_keywords(&parsed.semantic_prompt, parsed, limit) {
            merge_keywords(&mut hits, words);
        }
        hits.sort_by(|left, right| {
            left.distance
                .partial_cmp(&right.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit);
        Ok(hits)
    }

    fn search_knn(&self, query_vec: &[f32; EMBED_DIM], limit: usize) -> Result<Vec<SearchHit>> {
        let k_candidates = (limit * 8).max(100) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT knn.blake3_hash, fp.rel_path, knn.distance,
                    ex.focal_length_mm, ex.aperture_f, ex.camera_model
             FROM (
                SELECT blake3_hash, distance
                FROM vec_media
                WHERE embedding MATCH ?1 AND k = ?2
             ) knn
             JOIN file_paths fp ON fp.blake3_hash = knn.blake3_hash
             LEFT JOIN exif_metadata ex ON ex.blake3_hash = knn.blake3_hash
             ORDER BY knn.distance ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![query_vec.as_bytes(), k_candidates, limit as i64],
            read_hit,
        )?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Score every photo that already matches the camera filters. The nearest-k
    /// probe drops those photos when the filter is selective.
    fn score_filtered(
        &self,
        query_vec: &[f32; EMBED_DIM],
        parsed: &ParsedQuery,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let mut sql = String::from(
            "SELECT v.blake3_hash, v.embedding, fp.rel_path,
                    ex.focal_length_mm, ex.aperture_f, ex.camera_model
             FROM vec_media v
             JOIN file_paths fp ON fp.blake3_hash = v.blake3_hash
             LEFT JOIN exif_metadata ex ON ex.blake3_hash = v.blake3_hash
             WHERE 1=1 ",
        );
        push_exif_filters(&mut sql, parsed);
        sql.push_str(" LIMIT 8000");

        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let mut hits = Vec::new();
        while let Some(row) = rows.next()? {
            let blob: Vec<u8> = row.get(1)?;
            let Some(embedding) = parse_embedding(&blob) else {
                continue;
            };
            hits.push(SearchHit {
                blake3_hash: row.get(0)?,
                rel_path: row.get(2)?,
                distance: cosine_distance(query_vec, &embedding),
                focal_length_mm: row.get(3)?,
                aperture_f: row.get(4)?,
                camera_model: row.get(5)?,
            });
        }
        hits.sort_by(|left, right| {
            left.distance
                .partial_cmp(&right.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit);
        Ok(hits)
    }

    fn search_keywords(
        &self,
        prompt: &str,
        parsed: &ParsedQuery,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let Some(match_query) = fts_query(prompt) else {
            return Ok(Vec::new());
        };
        let mut sql = String::from(
            "SELECT f.blake3_hash, fp.rel_path, 0.55,
                    ex.focal_length_mm, ex.aperture_f, ex.camera_model
             FROM media_fts f
             JOIN file_paths fp ON fp.blake3_hash = f.blake3_hash
             LEFT JOIN exif_metadata ex ON ex.blake3_hash = f.blake3_hash
             WHERE media_fts MATCH ?1 ",
        );
        push_exif_filters(&mut sql, parsed);
        sql.push_str(" LIMIT ?2");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![match_query, limit as i64], read_hit)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }
}

fn write_fts(conn: &Connection, blake3_hash: &str, extra: Option<&str>) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT fp.rel_path, ex.camera_make, ex.camera_model, ex.lens_model
         FROM file_paths fp
         LEFT JOIN exif_metadata ex ON ex.blake3_hash = fp.blake3_hash
         WHERE fp.blake3_hash = ?1",
    )?;
    let parts: Vec<String> = stmt
        .query_map(params![blake3_hash], |row| {
            let rel_path: String = row.get(0)?;
            let make: Option<String> = row.get(1)?;
            let model: Option<String> = row.get(2)?;
            let lens: Option<String> = row.get(3)?;
            Ok([Some(rel_path), make, model, lens]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" "))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(stmt);
    conn.execute(
        "DELETE FROM media_fts WHERE blake3_hash = ?1",
        params![blake3_hash],
    )?;
    let mut body = parts.join(" ");
    if let Some(extra) = extra {
        if !extra.trim().is_empty() {
            body.push(' ');
            body.push_str(extra);
        }
    }
    if !body.trim().is_empty() {
        conn.execute(
            "INSERT INTO media_fts (blake3_hash, body) VALUES (?1, ?2)",
            params![blake3_hash, body],
        )?;
    }
    Ok(())
}

fn push_exif_filters(sql: &mut String, parsed: &ParsedQuery) {
    if let Some(focal) = parsed.focal_length_mm {
        sql.push_str(&format!(
            " AND ABS(ex.focal_length_mm - {focal}) < 1.0"
        ));
    }
    if let Some(aperture) = parsed.max_aperture_f {
        sql.push_str(&format!(" AND ex.aperture_f <= {}", aperture + 0.05));
    }
    if let Some(iso) = parsed.max_iso {
        sql.push_str(&format!(" AND ex.iso <= {iso}"));
    }
    if let Some((start_ts, end_ts)) = parsed.year_range {
        sql.push_str(&format!(
            " AND ex.captured_at BETWEEN {start_ts} AND {end_ts}"
        ));
    }
}

fn fts_query(prompt: &str) -> Option<String> {
    let mut parts = Vec::new();
    for token in prompt.split(|c: char| !c.is_alphanumeric()) {
        if token.len() < 2 || token.contains('"') {
            continue;
        }
        parts.push(format!("\"{token}\""));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" OR "))
    }
}

fn read_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<SearchHit> {
    Ok(SearchHit {
        blake3_hash: row.get(0)?,
        rel_path: row.get(1)?,
        distance: row.get(2)?,
        focal_length_mm: row.get(3)?,
        aperture_f: row.get(4)?,
        camera_model: row.get(5)?,
    })
}

fn parse_embedding(blob: &[u8]) -> Option<[f32; EMBED_DIM]> {
    if blob.len() != EMBED_DIM * 4 {
        return None;
    }
    let mut embedding = [0.0f32; EMBED_DIM];
    for (index, chunk) in blob.chunks_exact(4).enumerate() {
        embedding[index] = f32::from_le_bytes(chunk.try_into().ok()?);
    }
    Some(embedding)
}

fn cosine_distance(left: &[f32; EMBED_DIM], right: &[f32; EMBED_DIM]) -> f64 {
    let mut dot = 0.0f32;
    for index in 0..EMBED_DIM {
        dot += left[index] * right[index];
    }
    (1.0 - dot) as f64
}

fn merge_keywords(hits: &mut Vec<SearchHit>, words: Vec<SearchHit>) {
    for word in words {
        if let Some(existing) = hits.iter_mut().find(|hit| hit.rel_path == word.rel_path) {
            existing.distance -= 0.12;
        } else {
            hits.push(word);
        }
    }
}
