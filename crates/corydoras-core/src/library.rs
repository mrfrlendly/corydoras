use crate::analysis;
use crate::mesh;
use crate::palette;
use crate::render;
use crate::spec;
use crate::sources::{self, Extracted};
use crate::db;
use crate::models::*;
use crate::util::*;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Row};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub const JOB_ANALYZE: &str = "analyze_item";
pub const JOB_SOURCE: &str = "analyze_source";
pub const JOB_SYNTH: &str = "synthesize";

/// Delay before an item is analyzed again after an annotation change (seconds).
const REANALYZE_DELAY: i64 = 8;
/// Largest file that the app imports (bytes).
const MAX_IMPORT_BYTES: usize = 80 * 1024 * 1024;

/// Analysis jobs (images and sources) that wait for one language.
const PENDING_SQL: &str = "SELECT COUNT(*) FROM jobs j WHERE
    (j.kind = 'analyze_item' AND j.target IN (SELECT id FROM items WHERE language_id = ?1))
    OR (j.kind = 'analyze_source' AND j.target IN (SELECT id FROM sources WHERE language_id = ?1))";

const ITEM_COLS: &str = "i.id, i.language_id, i.kind, i.file_name, i.original_name, i.source, \
    i.width, i.height, i.added_at, i.tags, i.note, i.weight, i.anti, i.analysis_status, \
    i.analysis_error, (SELECT COUNT(*) FROM regions r WHERE r.item_id = i.id), \
    i.canvas_x, i.canvas_y, i.canvas_w, i.model_file, i.palette";

fn item_from_row(r: &Row) -> rusqlite::Result<Item> {
    let id: String = r.get(0)?;
    let language_id: String = r.get(1)?;
    let file_name: String = r.get(3)?;
    let tags_json: String = r.get(9)?;
    let model_file: Option<String> = r.get(19)?;
    let palette_json: Option<String> = r.get(20)?;
    Ok(Item {
        model_rel: model_file.map(|m| format!("languages/{language_id}/items/{m}")),
        palette: palette_json.and_then(|p| serde_json::from_str(&p).ok()).unwrap_or_default(),
        file_rel: format!("languages/{language_id}/items/{file_name}"),
        thumb_rel: format!("languages/{language_id}/thumbs/{id}.jpg"),
        id,
        language_id,
        kind: r.get(2)?,
        original_name: r.get(4)?,
        source: r.get(5)?,
        width: r.get(6)?,
        height: r.get(7)?,
        added_at: r.get(8)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        note: r.get(10)?,
        weight: r.get(11)?,
        anti: r.get::<_, i64>(12)? != 0,
        analysis_status: r.get(13)?,
        analysis_error: r.get(14)?,
        region_count: r.get(15)?,
        canvas: match (r.get::<_, Option<f64>>(16)?, r.get::<_, Option<f64>>(17)?, r.get::<_, Option<f64>>(18)?) {
            (Some(x), Some(y), Some(w)) => Some(CanvasPos { x, y, w }),
            _ => None,
        },
    })
}

fn region_from_row(r: &Row) -> rusqlite::Result<Region> {
    Ok(Region {
        id: r.get(0)?,
        x: r.get(1)?,
        y: r.get(2)?,
        w: r.get(3)?,
        h: r.get(4)?,
        note: r.get(5)?,
        tag: r.get(6)?,
    })
}

fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim().to_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// One library folder: the SQLite index plus the files of each language.
pub struct Library {
    root: PathBuf,
    conn: Mutex<Connection>,
}

impl Library {
    /// Opens the library at `root`. Makes the folder and the database if necessary.
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root.join("languages"))
            .with_context(|| format!("Cannot make the library folder {}", root.display()))?;
        let conn = db::open(&root.join("library.sqlite"))?;
        let lib = Self {
            root: root.to_path_buf(),
            conn: Mutex::new(conn),
        };
        lib.migrate_rule_sets()?;
        lib.seed_rule_sets()?;
        lib.seed_print_log()?;
        Ok(lib)
    }

    /// Opens the library for reads only (used by the MCP server).
    pub fn open_query_only(root: &Path) -> Result<Self> {
        let lib = Self::open(root)?;
        lib.c().execute_batch("PRAGMA query_only=ON;")?;
        Ok(lib)
    }

    /// Reads an existing SQLite index without migrations or default records.
    pub fn open_read_only(root: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            root.join("library.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        conn.execute_batch("PRAGMA query_only=ON;")?;
        Ok(Self {
            root: root.to_path_buf(),
            conn: Mutex::new(conn),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Writes a consistent copy of the database to `path` (the WAL included).
    pub fn copy_database_to(&self, path: &Path) -> Result<()> {
        if path.exists() {
            anyhow::bail!("{} exists already.", path.display());
        }
        self.c().execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
        Ok(())
    }

    /// True when the database passes SQLite's integrity check.
    pub fn integrity_ok(&self) -> Result<bool> {
        let r: String = self.c().query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        Ok(r == "ok")
    }

    pub(crate) fn c(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn lang_dir(&self, language_id: &str) -> PathBuf {
        self.root.join("languages").join(language_id)
    }

    pub fn abs(&self, rel: &str) -> Result<PathBuf> {
        safe_join(&self.root, rel).ok_or_else(|| anyhow!("Bad path: {rel}"))
    }

    // ------------------------------------------------------------------
    // Languages
    // ------------------------------------------------------------------

    pub fn list_languages(&self) -> Result<Vec<Language>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT l.id, l.name, l.description, l.created_at, l.updated_at,
                    (SELECT COUNT(*) FROM items i WHERE i.language_id = l.id),
                    (SELECT MAX(version) FROM versions v WHERE v.language_id = l.id),
                    (SELECT COUNT(*) FROM sources s WHERE s.language_id = l.id),
                    l.base_id, l.base_version, l.overrides
             FROM languages l ORDER BY l.name COLLATE NOCASE",
        )?;
        let rows = st
            .query_map([], |r| {
                let overrides: String = r.get(10)?;
                Ok(Language {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    created_at: r.get(3)?,
                    updated_at: r.get(4)?,
                    item_count: r.get(5)?,
                    latest_version: r.get(6)?,
                    source_count: r.get(7)?,
                    base_id: r.get(8)?,
                    base_version: r.get(9)?,
                    override_count: serde_json::from_str::<Vec<Value>>(&overrides).map(|v| v.len() as i64).unwrap_or(0),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_language(&self, id: &str) -> Result<Language> {
        self.list_languages()?
            .into_iter()
            .find(|l| l.id == id)
            .ok_or_else(|| anyhow!("No language with ID \"{id}\"."))
    }

    /// Finds a language by ID or by name (case-insensitive).
    pub fn find_language(&self, key: &str) -> Result<Language> {
        let key = key.trim();
        let all = self.list_languages()?;
        if let Some(l) = all.iter().find(|l| l.id == key) {
            return Ok(l.clone());
        }
        if let Some(l) = all.iter().find(|l| l.name.eq_ignore_ascii_case(key)) {
            return Ok(l.clone());
        }
        let slug = slugify(key);
        all.into_iter()
            .find(|l| l.id == slug)
            .ok_or_else(|| anyhow!("No language matches \"{key}\". Call list_languages to see the IDs."))
    }

    pub fn create_language(&self, name: &str, description: &str) -> Result<Language> {
        let name = name.trim();
        if name.is_empty() {
            bail!("The language needs a name.");
        }
        let base = slugify(name);
        let id = {
            let c = self.c();
            let mut id = base.clone();
            let mut n = 2;
            while c
                .query_row("SELECT 1 FROM languages WHERE id = ?1", [&id], |_| Ok(()))
                .optional()?
                .is_some()
            {
                id = format!("{base}-{n}");
                n += 1;
            }
            let now = now_iso();
            c.execute(
                "INSERT INTO languages (id, name, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![id, name, description.trim(), now],
            )?;
            id
        };
        let dir = self.lang_dir(&id);
        for sub in ["items", "thumbs", "crops", "versions"] {
            std::fs::create_dir_all(dir.join(sub))?;
        }
        self.get_language(&id)
    }

    pub fn update_language(&self, id: &str, name: &str, description: &str) -> Result<Language> {
        let name = name.trim();
        if name.is_empty() {
            bail!("The language needs a name.");
        }
        let changed = self.c().execute(
            "UPDATE languages SET name = ?2, description = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, name, description.trim(), now_iso()],
        )?;
        if changed == 0 {
            bail!("No language with ID \"{id}\".");
        }
        self.enqueue_synth(id)?;
        self.get_language(id)
    }

    fn touch_language(&self, id: &str) -> Result<()> {
        self.c().execute(
            "UPDATE languages SET updated_at = ?2 WHERE id = ?1",
            params![id, now_iso()],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Items
    // ------------------------------------------------------------------

    pub fn list_items(&self, language_id: &str) -> Result<Vec<Item>> {
        let c = self.c();
        let sql = format!(
            "SELECT {ITEM_COLS} FROM items i WHERE i.language_id = ?1 ORDER BY i.added_at DESC, i.rowid DESC"
        );
        let mut st = c.prepare(&sql)?;
        let rows = st
            .query_map([language_id], item_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_item(&self, id: &str) -> Result<Item> {
        let c = self.c();
        let sql = format!("SELECT {ITEM_COLS} FROM items i WHERE i.id = ?1");
        c.query_row(&sql, [id], item_from_row)
            .optional()?
            .ok_or_else(|| anyhow!("No item with ID \"{id}\"."))
    }

    pub fn get_regions(&self, item_id: &str) -> Result<Vec<Region>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, x, y, w, h, note, tag FROM regions WHERE item_id = ?1 ORDER BY ord, rowid",
        )?;
        let rows = st
            .query_map([item_id], region_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_analysis(&self, item_id: &str) -> Result<Option<(Value, String)>> {
        let row: Option<(String, String)> = self
            .c()
            .query_row(
                "SELECT json, model FROM analyses WHERE item_id = ?1",
                [item_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.map(|(j, m)| (serde_json::from_str(&j).unwrap_or(Value::Null), m)))
    }

    pub fn item_detail(&self, id: &str) -> Result<ItemDetail> {
        let item = self.get_item(id)?;
        let regions = self.get_regions(id)?;
        let analysis = self.get_analysis(id)?;
        Ok(ItemDetail {
            item,
            regions,
            analysis_model: analysis.as_ref().map(|a| a.1.clone()),
            analysis: analysis.map(|a| a.0),
        })
    }

    /// Imports one file from memory. Returns the existing item if the same
    /// file is already in this language.
    pub fn import_bytes(
        &self,
        language_id: &str,
        original_name: &str,
        bytes: &[u8],
        source: Option<&str>,
    ) -> Result<Item> {
        self.get_language(language_id)?;
        if bytes.is_empty() {
            bail!("The file \"{original_name}\" is empty.");
        }
        if bytes.len() > MAX_IMPORT_BYTES {
            bail!("The file \"{original_name}\" is larger than 80 MB.");
        }
        let format = match image::guess_format(bytes) {
            Ok(f) => f,
            Err(_) => {
                let lower = original_name.to_lowercase();
                if crate::video::is_video_name(&lower) {
                    bail!("\"{original_name}\" is a video. Add it with Add images… or drag it from Finder, and the app takes its key frames.");
                }
                if lower.ends_with(".heic") || lower.ends_with(".heif") {
                    bail!("HEIC is not supported yet. Export \"{original_name}\" as JPEG or PNG.");
                }
                bail!("\"{original_name}\" is not an image type that Corydoras can read.");
            }
        };

        let hash = fnv1a64(bytes);
        let existing: Option<String> = self
            .c()
            .query_row(
                "SELECT id FROM items WHERE language_id = ?1 AND hash = ?2",
                params![language_id, hash],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return self.get_item(&id);
        }

        let img = analysis::decode_oriented(bytes)
            .with_context(|| format!("Cannot read the image \"{original_name}\""))?;
        let id = uuid::Uuid::new_v4().to_string();
        let ext = format.extensions_str().first().copied().unwrap_or("img");
        let file_name = format!("{id}.{ext}");
        let dir = self.lang_dir(language_id);
        std::fs::create_dir_all(dir.join("items"))?;
        std::fs::create_dir_all(dir.join("thumbs"))?;
        std::fs::write(dir.join("items").join(&file_name), bytes)?;
        let thumb = analysis::encode_jpeg(&img, 640, 84)?;
        std::fs::write(dir.join("thumbs").join(format!("{id}.jpg")), thumb)?;

        let name = if original_name.trim().is_empty() {
            format!("image.{ext}")
        } else {
            original_name.trim().to_string()
        };
        self.c().execute(
            "INSERT INTO items (id, language_id, kind, file_name, original_name, source, width, height, hash, added_at, palette)
             VALUES (?1, ?2, 'image', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                language_id,
                file_name,
                name,
                source.map(|s| s.trim()).filter(|s| !s.is_empty()),
                img.width() as i64,
                img.height() as i64,
                hash,
                now_iso(),
                serde_json::to_string(&palette::of_image(&img))?
            ],
        )?;
        self.touch_language(language_id)?;
        self.enqueue(JOB_ANALYZE, &id, 0)?;
        self.enqueue_synth(language_id)?;
        self.get_item(&id)
    }

    /// Adds a 3D model (STL, 3MF, OBJ or STEP) as a reference. The app draws
    /// four views of it into one image; that image is what the board shows
    /// and what the analysis reads. The model file is kept beside it. `name`
    /// is the file name of the model, `source` where it came from.
    pub fn import_model(&self, language_id: &str, path: &Path, name: &str, source: Option<&str>) -> Result<Item> {
        self.get_language(language_id)?;
        let name = name.trim();
        if !mesh::is_model_name(name) {
            bail!("\"{name}\" is not a model type that Corydoras reads. Use STL, 3MF, OBJ or STEP.");
        }
        let bytes = std::fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;
        let hash = format!("model-{}", fnv1a64(&bytes));
        let existing: Option<String> = self
            .c()
            .query_row("SELECT id FROM items WHERE language_id = ?1 AND hash = ?2", params![language_id, hash], |r| r.get(0))
            .optional()?;
        if let Some(id) = existing {
            return self.get_item(&id);
        }
        // The reader goes by the file name, and a dropped model arrives under a temporary name.
        let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let id = uuid::Uuid::new_v4().to_string();
        let dir = self.lang_dir(language_id);
        std::fs::create_dir_all(dir.join("items"))?;
        std::fs::create_dir_all(dir.join("thumbs"))?;
        let model_file = format!("{id}.model.{ext}");
        let stored = dir.join("items").join(&model_file);
        std::fs::write(&stored, &bytes)?;
        let made = (|| -> Result<(Vec<u8>, image::DynamicImage, usize)> {
            let loaded = mesh::load(&stored).with_context(|| format!("Cannot read the model \"{name}\""))?;
            let png = render::sheet_png(&loaded.mesh)?;
            let img = analysis::decode_oriented(&png)?;
            Ok((png, img, loaded.mesh.tris.len()))
        })();
        let (png, img, _triangles) = match made {
            Ok(x) => x,
            Err(e) => {
                let _ = std::fs::remove_file(&stored);
                return Err(e);
            }
        };
        let file_name = format!("{id}.png");
        std::fs::write(dir.join("items").join(&file_name), &png)?;
        std::fs::write(dir.join("thumbs").join(format!("{id}.jpg")), analysis::encode_jpeg(&img, 640, 84)?)?;
        self.c().execute(
            "INSERT INTO items (id, language_id, kind, file_name, original_name, source, width, height, hash, added_at, model_file, palette)
             VALUES (?1, ?2, 'model', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '[]')",
            params![
                id,
                language_id,
                file_name,
                name,
                source.map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or(name),
                img.width() as i64,
                img.height() as i64,
                hash,
                now_iso(),
                model_file
            ],
        )?;
        self.touch_language(language_id)?;
        self.enqueue(JOB_ANALYZE, &id, 0)?;
        self.enqueue_synth(language_id)?;
        self.get_item(&id)
    }

    /// True when the language has an image with this content hash already.
    pub fn has_item_hash(&self, language_id: &str, hash: &str) -> Result<bool> {
        Ok(self
            .c()
            .query_row("SELECT 1 FROM items WHERE language_id = ?1 AND hash = ?2", params![language_id, hash], |_| Ok(()))
            .optional()?
            .is_some())
    }

    /// Finds the palettes of items that have none yet (items of older
    /// versions). At most `limit` items in one call. Returns how many it did.
    pub fn backfill_palettes(&self, language_id: &str, limit: usize) -> Result<usize> {
        let todo: Vec<(String, String)> = {
            let c = self.c();
            let mut st = c.prepare("SELECT id, kind FROM items WHERE language_id = ?1 AND palette IS NULL LIMIT ?2")?;
            let rows = st.query_map(params![language_id, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for (id, kind) in &todo {
            let json = if kind == "model" {
                "[]".to_string()
            } else {
                // The thumbnail is enough for the main colours, and it is small.
                let thumb = self.lang_dir(language_id).join("thumbs").join(format!("{id}.jpg"));
                match std::fs::read(&thumb).ok().and_then(|b| analysis::decode_oriented(&b).ok()) {
                    Some(img) => serde_json::to_string(&palette::of_image(&img))?,
                    None => "[]".to_string(),
                }
            };
            self.c().execute("UPDATE items SET palette = ?2 WHERE id = ?1", params![id, json])?;
        }
        Ok(todo.len())
    }

    /// The main colours of a board: the palettes of its references, each
    /// counted by the weight of its item. Anti-references do not count.
    pub fn language_palette(&self, language_id: &str) -> Result<Vec<palette::Swatch>> {
        let items = self.list_items(language_id)?;
        let parts: Vec<(Vec<palette::Swatch>, f64)> =
            items.into_iter().filter(|i| !i.anti && !i.palette.is_empty()).map(|i| (i.palette, i.weight as f64)).collect();
        Ok(palette::merged(&parts))
    }

    /// Takes the key frames of a video and adds each one as an item. `name` is the
    /// video's file name, `source` where it came from (a path or a link). Each
    /// frame is named and sourced with its position, for example "clip.mov · 0:12".
    pub fn import_video(&self, language_id: &str, path: &Path, name: &str, source: Option<&str>) -> Result<VideoImport> {
        self.get_language(language_id)?;
        let x = crate::video::extract(path, crate::video::MAX_FRAMES)?;
        let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into());
        let source = source.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(name);
        let mut items = Vec::new();
        for f in &x.frames {
            let tc = crate::video::timecode(f.time);
            let item = self.import_bytes(
                language_id,
                &format!("{stem} · {tc}.jpg"),
                &f.jpeg,
                Some(&format!("{source} · {tc}")),
            )?;
            items.push(item);
        }
        Ok(VideoImport { items, duration: x.duration })
    }

    /// Imports a file from disk. The source is the file path.
    pub fn import_path(&self, language_id: &str, path: &Path) -> Result<Item> {
        let bytes = std::fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.import_bytes(language_id, &name, &bytes, Some(&path.to_string_lossy()))
    }

    pub fn update_item(
        &self,
        id: &str,
        tags: &[String],
        note: &str,
        weight: i64,
        anti: bool,
    ) -> Result<Item> {
        let before = self.get_item(id)?;
        let tags = clean_tags(tags);
        let weight = weight.clamp(1, 5);
        self.c().execute(
            "UPDATE items SET tags = ?2, note = ?3, weight = ?4, anti = ?5 WHERE id = ?1",
            params![id, serde_json::to_string(&tags)?, note.trim(), weight, anti as i64],
        )?;
        // Tags, notes and the anti flag change what the analysis looks for.
        if before.tags != tags || before.note != note.trim() || before.anti != anti {
            self.set_item_status(id, "pending", None)?;
            self.enqueue(JOB_ANALYZE, id, REANALYZE_DELAY)?;
        }
        self.touch_language(&before.language_id)?;
        self.enqueue_synth(&before.language_id)?;
        self.get_item(id)
    }

    /// Saves canvas places. Only items of `language_id` change; unknown IDs are
    /// skipped. Places are layout, not references: no analysis or synthesis runs.
    /// Returns the number of items that moved.
    pub fn set_canvas(&self, language_id: &str, places: &[CanvasPlacement]) -> Result<usize> {
        let mut c = self.c();
        let tx = c.transaction()?;
        let mut n = 0;
        {
            let mut st = tx.prepare(
                "UPDATE items SET canvas_x = ?3, canvas_y = ?4, canvas_w = ?5 WHERE id = ?1 AND language_id = ?2",
            )?;
            for p in places {
                if !(p.x.is_finite() && p.y.is_finite() && p.w.is_finite()) || p.w <= 0.0 {
                    bail!("Bad canvas place for item {}.", p.id);
                }
                n += st.execute(params![p.id, language_id, p.x, p.y, p.w.clamp(40.0, 4000.0)])?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Forgets all canvas places of a language, so the canvas lays the items out again.
    pub fn clear_canvas(&self, language_id: &str) -> Result<()> {
        self.c().execute(
            "UPDATE items SET canvas_x = NULL, canvas_y = NULL, canvas_w = NULL WHERE language_id = ?1",
            [language_id],
        )?;
        Ok(())
    }

    pub fn set_regions(&self, item_id: &str, regions: &[RegionInput]) -> Result<Vec<Region>> {
        let item = self.get_item(item_id)?;
        {
            let mut c = self.c();
            let tx = c.transaction()?;
            tx.execute("DELETE FROM regions WHERE item_id = ?1", [item_id])?;
            for (i, r) in regions.iter().enumerate() {
                let x = r.x.clamp(0.0, 1.0);
                let y = r.y.clamp(0.0, 1.0);
                let w = r.w.clamp(0.0, 1.0 - x);
                let h = r.h.clamp(0.0, 1.0 - y);
                if w < 0.005 || h < 0.005 {
                    continue;
                }
                let id = r
                    .id
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                tx.execute(
                    "INSERT INTO regions (id, item_id, ord, x, y, w, h, note, tag) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![id, item_id, i as i64, x, y, w, h, r.note.trim(), r.tag.trim().to_lowercase()],
                )?;
            }
            tx.commit()?;
        }
        self.set_item_status(item_id, "pending", None)?;
        self.enqueue(JOB_ANALYZE, item_id, REANALYZE_DELAY)?;
        self.enqueue_synth(&item.language_id)?;
        self.get_regions(item_id)
    }

    /// Removes an item from the language. The files move to the "_removed"
    /// folder of the language; nothing is deleted from disk.
    pub fn remove_item(&self, id: &str) -> Result<()> {
        let item = self.get_item(id)?;
        let removed = self.lang_dir(&item.language_id).join("_removed");
        std::fs::create_dir_all(&removed)?;
        let mut files = vec![item.file_rel.clone(), item.thumb_rel.clone()];
        files.extend(item.model_rel.clone());
        for rel in &files {
            let src = self.abs(rel)?;
            if src.exists() {
                let name = src.file_name().map(|n| n.to_owned()).unwrap_or_default();
                let mut dest = removed.join(&name);
                if dest.exists() {
                    dest = removed.join(format!("{}-{}", now_ts(), name.to_string_lossy()));
                }
                std::fs::rename(&src, &dest)?;
            }
        }
        {
            let c = self.c();
            c.execute(
                "DELETE FROM jobs WHERE kind = ?1 AND target = ?2",
                params![JOB_ANALYZE, id],
            )?;
            c.execute("DELETE FROM items WHERE id = ?1", [id])?;
        }
        self.touch_language(&item.language_id)?;
        self.enqueue_synth(&item.language_id)?;
        Ok(())
    }

    pub fn reanalyze_item(&self, id: &str) -> Result<()> {
        let item = self.get_item(id)?;
        self.set_item_status(id, "pending", None)?;
        self.enqueue(JOB_ANALYZE, id, 0)?;
        self.enqueue_synth(&item.language_id)?;
        Ok(())
    }

    /// Queues every item of a language for analysis again (for example after a
    /// prompt upgrade). Returns the count.
    pub fn reanalyze_all(&self, language_id: &str) -> Result<usize> {
        let items = self.list_items(language_id)?;
        for i in &items {
            self.set_item_status(&i.id, "pending", None)?;
            self.enqueue(JOB_ANALYZE, &i.id, 0)?;
        }
        let srcs = self.list_sources(language_id)?;
        for x in &srcs {
            self.set_source_status(&x.id, "pending", None)?;
            self.enqueue(JOB_SOURCE, &x.id, 0)?;
        }
        if !items.is_empty() || !srcs.is_empty() {
            self.enqueue_synth(language_id)?;
        }
        Ok(items.len() + srcs.len())
    }

    /// Items of a language whose ID starts with one of the given prefixes
    /// (agents cite the first 8 characters). A language that extends a base
    /// cites the items of the base, so those count too.
    pub fn items_by_prefix(&self, language_id: &str, prefixes: &[String]) -> Result<Vec<Item>> {
        let mut items = Vec::new();
        for l in self.base_chain(language_id)? {
            items.extend(self.list_items(&l.id)?);
        }
        let mut out = Vec::new();
        for p in prefixes {
            let p = p.trim().trim_matches('`');
            if p.len() < 4 {
                continue;
            }
            if let Some(i) = items.iter().find(|i| i.id.starts_with(p)) {
                if !out.iter().any(|o: &Item| o.id == i.id) {
                    out.push(i.clone());
                }
            }
        }
        Ok(out)
    }

    /// Queues every failed item of a language for analysis again. Returns the count.
    pub fn retry_failed_items(&self, language_id: &str) -> Result<usize> {
        let failed: Vec<String> = self
            .list_items(language_id)?
            .into_iter()
            .filter(|i| i.analysis_status == "error")
            .map(|i| i.id)
            .collect();
        for id in &failed {
            self.set_item_status(id, "pending", None)?;
            self.enqueue(JOB_ANALYZE, id, 0)?;
        }
        let failed_src: Vec<String> = self
            .list_sources(language_id)?
            .into_iter()
            .filter(|x| x.analysis_status == "error")
            .map(|x| x.id)
            .collect();
        for id in &failed_src {
            self.set_source_status(id, "pending", None)?;
            self.enqueue(JOB_SOURCE, id, 0)?;
        }
        if !failed.is_empty() || !failed_src.is_empty() {
            self.enqueue_synth(language_id)?;
        }
        Ok(failed.len() + failed_src.len())
    }

    pub fn set_item_status(&self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        self.c().execute(
            "UPDATE items SET analysis_status = ?2, analysis_error = ?3 WHERE id = ?1",
            params![id, status, error],
        )?;
        Ok(())
    }

    pub fn save_analysis(&self, item_id: &str, json: &Value, model: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO analyses (item_id, json, model, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(item_id) DO UPDATE SET json = excluded.json, model = excluded.model, created_at = excluded.created_at",
            params![item_id, serde_json::to_string(json)?, model, now_iso()],
        )?;
        self.set_item_status(item_id, "done", None)
    }

    /// Analyzed items of a language, highest weight first (at most `limit`).
    pub fn synthesis_inputs(&self, language_id: &str, limit: usize) -> Result<Vec<SynthInput>> {
        let mut items = self.list_items(language_id)?;
        items.retain(|i| i.analysis_status == "done");
        items.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.added_at.cmp(&b.added_at)));
        items.truncate(limit);
        let mut out = Vec::new();
        for item in items {
            if let Some((analysis, _)) = self.get_analysis(&item.id)? {
                let regions = self.get_regions(&item.id)?;
                out.push(SynthInput {
                    item,
                    regions,
                    analysis,
                });
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // Sources (links and documents)
    // ------------------------------------------------------------------

    fn source_from_row(r: &Row) -> rusqlite::Result<Source> {
        let id: String = r.get(0)?;
        let language_id: String = r.get(1)?;
        let file_name: Option<String> = r.get(8)?;
        let has_preview: i64 = r.get(10)?;
        Ok(Source {
            file_rel: file_name.map(|f| format!("languages/{language_id}/sources/{f}")),
            preview_rel: (has_preview != 0).then(|| format!("languages/{language_id}/sources/{id}.jpg")),
            id,
            language_id,
            kind: r.get(2)?,
            title: r.get(3)?,
            url: r.get(4)?,
            site: r.get(5)?,
            description: r.get(6)?,
            media: r.get(7)?,
            original_name: r.get(9)?,
            text_chars: r.get(11)?,
            word_count: r.get(12)?,
            added_at: r.get(13)?,
            note: r.get(14)?,
            weight: r.get(15)?,
            analysis_status: r.get(16)?,
            analysis_error: r.get(17)?,
        })
    }

    const SOURCE_COLS: &'static str = "id, language_id, kind, title, url, site, description, media, file_name, original_name, \
        has_preview, text_chars, word_count, added_at, note, weight, analysis_status, analysis_error";

    pub fn list_sources(&self, language_id: &str) -> Result<Vec<Source>> {
        let c = self.c();
        let sql = format!("SELECT {} FROM sources WHERE language_id = ?1 ORDER BY added_at DESC, rowid DESC", Self::SOURCE_COLS);
        let mut st = c.prepare(&sql)?;
        let rows = st
            .query_map([language_id], Self::source_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_source(&self, id: &str) -> Result<Source> {
        let c = self.c();
        let sql = format!("SELECT {} FROM sources WHERE id = ?1", Self::SOURCE_COLS);
        c.query_row(&sql, [id], Self::source_from_row)
            .optional()?
            .ok_or_else(|| anyhow!("No source with ID \"{id}\"."))
    }

    fn sources_dir(&self, language_id: &str) -> PathBuf {
        self.lang_dir(language_id).join("sources")
    }

    /// The full extracted text of a source.
    pub fn source_text(&self, id: &str) -> Result<String> {
        let s = self.get_source(id)?;
        Ok(std::fs::read_to_string(self.sources_dir(&s.language_id).join(format!("{id}.txt"))).unwrap_or_default())
    }

    pub fn get_source_analysis(&self, id: &str) -> Result<Option<(Value, String)>> {
        let row: Option<(Option<String>, Option<String>)> = self
            .c()
            .query_row("SELECT analysis, analysis_model FROM sources WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(match row {
            Some((Some(j), m)) => Some((serde_json::from_str(&j).unwrap_or(Value::Null), m.unwrap_or_default())),
            _ => None,
        })
    }

    pub fn source_detail(&self, id: &str) -> Result<SourceDetail> {
        let source = self.get_source(id)?;
        let text = self.source_text(id)?;
        let analysis = self.get_source_analysis(id)?;
        Ok(SourceDetail {
            source,
            analysis_model: analysis.as_ref().map(|a| a.1.clone()),
            analysis: analysis.map(|a| a.0),
            text_preview: text.chars().take(8000).collect(),
        })
    }

    /// Stores a fetched link or a read document and queues its analysis.
    fn insert_source(
        &self,
        language_id: &str,
        kind: &str,
        ex: &Extracted,
        url: Option<&str>,
        original_name: &str,
        file: Option<(&str, &[u8])>,
        hash: &str,
    ) -> Result<Source> {
        let id = uuid::Uuid::new_v4().to_string();
        let dir = self.sources_dir(language_id);
        std::fs::create_dir_all(&dir)?;
        let file_name = match file {
            Some((name, bytes)) => {
                let ext = sources::extension(name);
                let f = if ext.is_empty() { id.clone() } else { format!("{id}.{ext}") };
                std::fs::write(dir.join(&f), bytes)?;
                Some(f)
            }
            None => None,
        };
        // A document preview needs the saved file (Quick Look on macOS).
        let preview = match (&ex.preview, &file_name) {
            (Some(p), _) => Some(p.clone()),
            (None, Some(f)) => sources::quicklook_preview(&dir.join(f)),
            _ => None,
        };
        if let Some(p) = &preview {
            std::fs::write(dir.join(format!("{id}.jpg")), p)?;
        }
        std::fs::write(dir.join(format!("{id}.txt")), &ex.text)?;
        self.c().execute(
            "INSERT INTO sources (id, language_id, kind, title, url, site, description, media, file_name, original_name,
                                  has_preview, text_chars, word_count, hash, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id,
                language_id,
                kind,
                ex.title,
                url,
                ex.site,
                ex.description,
                ex.media,
                file_name,
                original_name,
                preview.is_some() as i64,
                ex.text.chars().count() as i64,
                sources::word_count(&ex.text),
                hash,
                now_iso()
            ],
        )?;
        self.touch_language(language_id)?;
        self.enqueue(JOB_SOURCE, &id, 0)?;
        self.enqueue_synth(language_id)?;
        self.get_source(&id)
    }

    fn find_source_by(&self, language_id: &str, column: &str, value: &str) -> Result<Option<Source>> {
        let id: Option<String> = self
            .c()
            .query_row(
                &format!("SELECT id FROM sources WHERE language_id = ?1 AND {column} = ?2"),
                params![language_id, value],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|i| self.get_source(&i)).transpose()
    }

    /// Adds a link that `sources::fetch_link` read. Returns the existing source
    /// if the same link is already in this language.
    pub fn add_link_source(&self, language_id: &str, url: &str, ex: &Extracted) -> Result<Source> {
        self.get_language(language_id)?;
        if let Some(s) = self.find_source_by(language_id, "url", url.trim())? {
            return Ok(s);
        }
        let file = ex.file.as_ref().map(|(n, b)| (n.as_str(), b.as_slice()));
        let hash = ex.file.as_ref().map(|(_, b)| fnv1a64(b)).unwrap_or_default();
        self.insert_source(language_id, "link", ex, Some(url.trim()), url.trim(), file, &hash)
    }

    /// Imports a document from memory. Returns the existing source if the same
    /// file is already in this language.
    pub fn import_document(&self, language_id: &str, name: &str, bytes: &[u8]) -> Result<Source> {
        self.get_language(language_id)?;
        let hash = fnv1a64(bytes);
        if let Some(s) = self.find_source_by(language_id, "hash", &hash)? {
            return Ok(s);
        }
        let ex = sources::extract_document(name, bytes, None)?;
        self.insert_source(language_id, "document", &ex, None, name, Some((name, bytes)), &hash)
    }

    pub fn update_source(&self, id: &str, title: &str, note: &str, weight: i64) -> Result<Source> {
        let before = self.get_source(id)?;
        let title = if title.trim().is_empty() { before.title.clone() } else { title.trim().to_string() };
        self.c().execute(
            "UPDATE sources SET title = ?2, note = ?3, weight = ?4 WHERE id = ?1",
            params![id, title, note.trim(), weight.clamp(1, 5)],
        )?;
        // The note steers what the analysis looks for.
        if before.note != note.trim() {
            self.set_source_status(id, "pending", None)?;
            self.enqueue(JOB_SOURCE, id, REANALYZE_DELAY)?;
        }
        self.enqueue_synth(&before.language_id)?;
        self.get_source(id)
    }

    /// Removes a source. Its files move to the "_removed" folder of the language.
    pub fn remove_source(&self, id: &str) -> Result<()> {
        let s = self.get_source(id)?;
        let dir = self.sources_dir(&s.language_id);
        let removed = self.lang_dir(&s.language_id).join("_removed");
        std::fs::create_dir_all(&removed)?;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with(id) {
                    let _ = std::fs::rename(e.path(), removed.join(&name));
                }
            }
        }
        {
            let c = self.c();
            c.execute("DELETE FROM jobs WHERE kind = ?1 AND target = ?2", params![JOB_SOURCE, id])?;
            c.execute("DELETE FROM sources WHERE id = ?1", [id])?;
        }
        self.touch_language(&s.language_id)?;
        self.enqueue_synth(&s.language_id)?;
        Ok(())
    }

    pub fn reanalyze_source(&self, id: &str) -> Result<()> {
        let s = self.get_source(id)?;
        self.set_source_status(id, "pending", None)?;
        self.enqueue(JOB_SOURCE, id, 0)?;
        self.enqueue_synth(&s.language_id)?;
        Ok(())
    }

    pub fn set_source_status(&self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        self.c().execute(
            "UPDATE sources SET analysis_status = ?2, analysis_error = ?3 WHERE id = ?1",
            params![id, status, error],
        )?;
        Ok(())
    }

    pub fn save_source_analysis(&self, id: &str, json: &Value, model: &str) -> Result<()> {
        self.c().execute(
            "UPDATE sources SET analysis = ?2, analysis_model = ?3, analysis_status = 'done', analysis_error = NULL WHERE id = ?1",
            params![id, serde_json::to_string(json)?, model],
        )?;
        Ok(())
    }

    pub fn mark_source_cancelled(&self, id: &str) -> Result<()> {
        if self.get_source_analysis(id)?.is_some() {
            self.set_source_status(id, "done", None)
        } else {
            self.set_source_status(id, "error", Some("Cancelled by you. Click Analyze again to run it."))
        }
    }

    /// Analyzed sources of a language, highest weight first.
    pub fn source_inputs(&self, language_id: &str, limit: usize) -> Result<Vec<SynthSource>> {
        let mut list = self.list_sources(language_id)?;
        list.retain(|s| s.analysis_status == "done");
        list.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.added_at.cmp(&b.added_at)));
        list.truncate(limit);
        let mut out = Vec::new();
        for s in list {
            if let Some((analysis, _)) = self.get_source_analysis(&s.id)? {
                out.push(SynthSource { source: s, analysis });
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // Versions
    // ------------------------------------------------------------------

    fn version_dir(&self, language_id: &str, version: i64) -> PathBuf {
        self.lang_dir(language_id)
            .join("versions")
            .join(format!("v{version:04}"))
    }

    pub fn list_versions(&self, language_id: &str) -> Result<Vec<VersionInfo>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT language_id, version, created_at, model, item_ids, edited, schema, warnings FROM versions
             WHERE language_id = ?1 ORDER BY version DESC",
        )?;
        let rows = st
            .query_map([language_id], |r| {
                let ids: String = r.get(4)?;
                let count = serde_json::from_str::<Vec<String>>(&ids)
                    .map(|v| v.len() as i64)
                    .unwrap_or(0);
                let warnings: String = r.get(7)?;
                Ok(VersionInfo {
                    language_id: r.get(0)?,
                    version: r.get(1)?,
                    created_at: r.get(2)?,
                    model: r.get(3)?,
                    item_count: count,
                    edited: r.get::<_, i64>(5)? != 0,
                    schema: r.get(6)?,
                    warnings: serde_json::from_str(&warnings).unwrap_or_default(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn latest_version(&self, language_id: &str) -> Result<Option<i64>> {
        Ok(self.c().query_row(
            "SELECT MAX(version) FROM versions WHERE language_id = ?1",
            [language_id],
            |r| r.get(0),
        )?)
    }

    pub fn latest_input_hash(&self, language_id: &str) -> Result<Option<String>> {
        Ok(self
            .c()
            .query_row(
                "SELECT input_hash FROM versions WHERE language_id = ?1 ORDER BY version DESC LIMIT 1",
                [language_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn read_version(&self, language_id: &str, version: i64) -> Result<VersionContent> {
        let info = self
            .list_versions(language_id)?
            .into_iter()
            .find(|v| v.version == version)
            .ok_or_else(|| anyhow!("Language \"{language_id}\" has no version {version}."))?;
        let dir = self.version_dir(language_id, version);
        let markdown = std::fs::read_to_string(dir.join("language.md")).unwrap_or_default();
        let tokens = std::fs::read_to_string(dir.join("tokens.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        Ok(VersionContent {
            info,
            markdown,
            tokens,
        })
    }

    pub fn read_latest(&self, language_id: &str) -> Result<Option<VersionContent>> {
        match self.latest_version(language_id)? {
            Some(v) => Ok(Some(self.read_version(language_id, v)?)),
            None => Ok(None),
        }
    }

    /// Writes a new schema 1 version (free markdown + tokens). Returns its number.
    pub fn save_version(
        &self,
        language_id: &str,
        markdown: &str,
        tokens: &Value,
        model: &str,
        item_ids: &[String],
        edited: bool,
        input_hash: &str,
    ) -> Result<i64> {
        let next = self.latest_version(language_id)?.unwrap_or(0) + 1;
        let mut tokens = tokens.clone();
        if let Some(obj) = tokens.as_object_mut() {
            obj.insert("language".into(), Value::String(language_id.to_string()));
            obj.insert("version".into(), Value::from(next));
        }
        self.write_version(language_id, next, markdown, &tokens, model, item_ids, edited, input_hash, 1, &[])?;
        Ok(next)
    }

    /// Writes a new schema 2 version. The app renders language.md from the spec.
    pub fn save_spec_version(
        &self,
        language_id: &str,
        spec_value: &Value,
        model: &str,
        item_ids: &[String],
        edited: bool,
        input_hash: &str,
        warnings: &[String],
    ) -> Result<i64> {
        let lang = self.get_language(language_id)?;
        let next = self.latest_version(language_id)?.unwrap_or(0) + 1;
        let mut tokens = spec_value.clone();
        spec::finalize(&mut tokens, language_id, next);
        let markdown = spec::render_markdown(&lang.name, &tokens, warnings);
        self.write_version(
            language_id,
            next,
            &markdown,
            &tokens,
            model,
            item_ids,
            edited,
            input_hash,
            spec::SCHEMA_VERSION,
            warnings,
        )?;
        Ok(next)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_version(
        &self,
        language_id: &str,
        version: i64,
        markdown: &str,
        tokens: &Value,
        model: &str,
        item_ids: &[String],
        edited: bool,
        input_hash: &str,
        schema: i64,
        warnings: &[String],
    ) -> Result<()> {
        let dir = self.version_dir(language_id, version);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("language.md"), markdown)?;
        std::fs::write(dir.join("tokens.json"), serde_json::to_string_pretty(tokens)?)?;
        self.c().execute(
            "INSERT INTO versions (language_id, version, created_at, model, item_ids, edited, input_hash, schema, warnings)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                language_id,
                version,
                now_iso(),
                model,
                serde_json::to_string(item_ids)?,
                edited as i64,
                input_hash,
                schema,
                serde_json::to_string(warnings)?
            ],
        )?;
        self.touch_language(language_id)?;
        // Languages that extend this one and follow its latest version get a new version too.
        self.rebuild_children(language_id)?;
        Ok(())
    }

    /// The 8-character IDs of the items and sources of a language: the evidence
    /// IDs that a spec may cite. A language that extends a base may cite the
    /// evidence of the base.
    pub fn known_evidence_ids(&self, language_id: &str) -> Result<Vec<String>> {
        let mut known: Vec<String> = Vec::new();
        for l in self.base_chain(language_id)? {
            known.extend(self.list_items(&l.id)?.iter().map(|i| i.id.chars().take(8).collect::<String>()));
            known.extend(self.list_sources(&l.id)?.iter().map(|x| x.id.chars().take(8).collect::<String>()));
        }
        Ok(known)
    }

    /// The input hash and item IDs that a manual version records.
    fn manual_version_inputs(&self, language_id: &str) -> Result<(String, Vec<String>)> {
        let inputs = self.synthesis_inputs(language_id, analysis::SYNTH_ITEM_LIMIT)?;
        let srcs = self.source_inputs(language_id, analysis::SYNTH_SOURCE_LIMIT)?;
        let hash = analysis::input_hash(&self.get_language(language_id)?, &inputs, &srcs);
        let ids: Vec<String> = inputs.iter().map(|i| i.item.id.clone()).collect();
        Ok((hash, ids))
    }

    /// Validates a schema 2 spec that a person (or an accepted proposal) wrote and
    /// saves it as a new edited version. `model` is the label of the version.
    pub fn save_manual_spec(&self, language_id: &str, tokens: &Value, model: &str) -> Result<i64> {
        // For a language that extends a base, the edit becomes overrides.
        if self.get_language(language_id)?.base_id.is_some() {
            return self.save_derived_edit(language_id, tokens);
        }
        let check = spec::validate(tokens, &self.known_evidence_ids(language_id)?);
        if !check.errors.is_empty() {
            bail!("tokens.json: {}", check.errors.join(" "));
        }
        let (hash, ids) = self.manual_version_inputs(language_id)?;
        self.save_spec_version(language_id, tokens, model, &ids, true, &hash, &check.warnings)
    }

    /// Saves your own edit as a new version. For a schema 2 spec, only
    /// tokens.json counts: the app renders language.md from it.
    pub fn save_edited_version(&self, language_id: &str, markdown: &str, tokens_json: &str) -> Result<i64> {
        let tokens: Value = serde_json::from_str(tokens_json)
            .map_err(|e| anyhow!("tokens.json is not valid JSON: {e}"))?;
        if spec::schema_of(&tokens) >= 2 {
            return self.save_manual_spec(language_id, &tokens, "edited by you");
        }
        let (hash, ids) = self.manual_version_inputs(language_id)?;
        analysis::validate_tokens(&tokens).map_err(|e| anyhow!("tokens.json: {e}"))?;
        self.save_version(language_id, markdown, &tokens, "edited by you", &ids, true, &hash)
    }

    // ------------------------------------------------------------------
    // Projects
    // ------------------------------------------------------------------

    fn pins_for(&self, project_id: &str) -> Result<Vec<Pin>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT p.language_id, l.name, p.version, p.role, p.scope,
                    (SELECT MAX(version) FROM versions v WHERE v.language_id = p.language_id)
             FROM pins p JOIN languages l ON l.id = p.language_id
             WHERE p.project_id = ?1
             ORDER BY CASE p.role WHEN 'primary' THEN 0 ELSE 1 END, l.name COLLATE NOCASE",
        )?;
        let rows = st
            .query_map([project_id], |r| {
                Ok(Pin {
                    language_id: r.get(0)?,
                    language_name: r.get(1)?,
                    version: r.get(2)?,
                    role: r.get(3)?,
                    scope: r.get(4)?,
                    latest_version: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let rows: Vec<(String, String, String)> = {
            let c = self.c();
            let mut st =
                c.prepare("SELECT id, name, created_at FROM projects ORDER BY name COLLATE NOCASE")?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut out = Vec::new();
        for (id, name, created_at) in rows {
            let pins = self.pins_for(&id)?;
            let rule_set_ids = self.rule_set_ids_for(&id)?;
            out.push(Project {
                id,
                name,
                created_at,
                pins,
                rule_set_ids,
            });
        }
        Ok(out)
    }

    /// Finds a project by ID or by name (case-insensitive).
    pub fn find_project(&self, key: &str) -> Result<Project> {
        let key = key.trim();
        self.list_projects()?
            .into_iter()
            .find(|p| p.id == key || p.name.eq_ignore_ascii_case(key))
            .ok_or_else(|| anyhow!("No project matches \"{key}\". Call list_projects to see the names."))
    }

    pub fn create_project(&self, name: &str) -> Result<Project> {
        let name = name.trim();
        if name.is_empty() {
            bail!("The project needs a name.");
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.c()
            .execute(
                "INSERT INTO projects (id, name, created_at) VALUES (?1, ?2, ?3)",
                params![id, name, now_iso()],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _)
                    if f.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    anyhow!("A project with the name \"{name}\" exists.")
                }
                other => other.into(),
            })?;
        self.find_project(&id)
    }

    pub fn rename_project(&self, id: &str, name: &str) -> Result<Project> {
        let name = name.trim();
        if name.is_empty() {
            bail!("The project needs a name.");
        }
        self.c()
            .execute("UPDATE projects SET name = ?2 WHERE id = ?1", params![id, name])?;
        self.find_project(id)
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        self.c().execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_pins(&self, project_id: &str, pins: &[PinInput]) -> Result<Project> {
        let primaries = pins.iter().filter(|p| p.role == "primary").count();
        if primaries > 1 {
            bail!("A project can have only one primary language.");
        }
        for p in pins {
            if !["primary", "secondary"].contains(&p.role.as_str()) {
                bail!("Unknown role \"{}\".", p.role);
            }
            if !["all", "cad", "cmf", "prompts"].contains(&p.scope.as_str()) {
                bail!("Unknown scope \"{}\".", p.scope);
            }
            let exists = self.list_versions(&p.language_id)?.iter().any(|v| v.version == p.version);
            if !exists {
                bail!("Language \"{}\" has no version {}.", p.language_id, p.version);
            }
        }
        {
            let mut c = self.c();
            let tx = c.transaction()?;
            tx.execute("DELETE FROM pins WHERE project_id = ?1", [project_id])?;
            for p in pins {
                tx.execute(
                    "INSERT OR REPLACE INTO pins (project_id, language_id, version, role, scope) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![project_id, p.language_id, p.version, p.role, p.scope],
                )?;
            }
            tx.commit()?;
        }
        self.find_project(project_id)
    }

    // ------------------------------------------------------------------
    // Rule sets
    // ------------------------------------------------------------------

    fn rule_set_from_row(r: &Row) -> rusqlite::Result<RuleSet> {
        let rules: String = r.get(4)?;
        Ok(RuleSet {
            id: r.get(0)?,
            name: r.get(1)?,
            description: r.get(2)?,
            always: r.get::<_, i64>(3)? != 0,
            rules: serde_json::from_str(&rules).unwrap_or_default(),
            created_at: r.get(5)?,
            updated_at: r.get(6)?,
            project_count: r.get(7)?,
        })
    }

    pub fn list_rule_sets(&self) -> Result<Vec<RuleSet>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT r.id, r.name, r.description, r.always, r.rules, r.created_at, r.updated_at,
                    (SELECT COUNT(*) FROM project_rule_sets p WHERE p.rule_set_id = r.id)
             FROM rule_sets r ORDER BY r.always DESC, r.name COLLATE NOCASE",
        )?;
        let rows = st
            .query_map([], Self::rule_set_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_rule_set(&self, key: &str) -> Result<RuleSet> {
        let key = key.trim();
        self.list_rule_sets()?
            .into_iter()
            .find(|p| p.id == key || p.name.eq_ignore_ascii_case(key))
            .ok_or_else(|| anyhow!("No rule set matches \"{key}\". Call list_rule_sets to see the names."))
    }

    /// Cleans the input: trims text, drops empty rules, drops a unit without a value.
    fn clean_rule_set(input: &RuleSetInput) -> Result<RuleSetInput> {
        let name = input.name.trim();
        if name.is_empty() {
            bail!("The rule set needs a name.");
        }
        let mut rules = Vec::new();
        for r in &input.rules {
            let rname = r.name.trim();
            if rname.is_empty() {
                if r.value.is_some() || !r.note.trim().is_empty() {
                    bail!("Each rule needs a name.");
                }
                continue;
            }
            let (value, unit) = match r.value {
                Some(v) if !v.is_finite() => bail!("Rule \"{rname}\": the value must be a number."),
                Some(v) => {
                    let u = r.unit.as_deref().unwrap_or("").trim().to_string();
                    if !spec::MEASURE_UNITS.contains(&u.as_str()) {
                        bail!("Rule \"{rname}\": a value needs a unit ({}).", spec::MEASURE_UNITS.join(", "));
                    }
                    (Some(v), Some(u))
                }
                None => (None, None),
            };
            rules.push(Rule { name: rname.to_string(), value, unit, note: r.note.trim().to_string() });
        }
        Ok(RuleSetInput {
            name: name.to_string(),
            description: input.description.trim().to_string(),
            always: input.always,
            rules,
        })
    }

    fn map_unique(e: rusqlite::Error, name: &str) -> anyhow::Error {
        match e {
            rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                anyhow!("A rule set with the name \"{name}\" exists.")
            }
            other => other.into(),
        }
    }

    pub fn create_rule_set(&self, input: &RuleSetInput) -> Result<RuleSet> {
        self.insert_rule_set(&uuid::Uuid::new_v4().to_string(), input)
    }

    fn insert_rule_set(&self, id: &str, input: &RuleSetInput) -> Result<RuleSet> {
        let input = Self::clean_rule_set(input)?;
        let now = now_iso();
        self.c()
            .execute(
                "INSERT INTO rule_sets (id, name, description, always, rules, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![id, input.name, input.description, input.always as i64, serde_json::to_string(&input.rules)?, now],
            )
            .map_err(|e| Self::map_unique(e, &input.name))?;
        self.get_rule_set(id)
    }

    pub fn update_rule_set(&self, id: &str, input: &RuleSetInput) -> Result<RuleSet> {
        let input = Self::clean_rule_set(input)?;
        let n = self
            .c()
            .execute(
                "UPDATE rule_sets SET name = ?2, description = ?3, always = ?4, rules = ?5, updated_at = ?6 WHERE id = ?1",
                params![id, input.name, input.description, input.always as i64, serde_json::to_string(&input.rules)?, now_iso()],
            )
            .map_err(|e| Self::map_unique(e, &input.name))?;
        if n == 0 {
            bail!("No rule set with ID \"{id}\".");
        }
        self.get_rule_set(id)
    }

    /// Deletes a rule set. Projects that selected it lose it.
    pub fn delete_rule_set(&self, id: &str) -> Result<()> {
        let c = self.c();
        c.execute("DELETE FROM project_rule_sets WHERE rule_set_id = ?1", [id])?;
        c.execute("DELETE FROM rule_sets WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The rule sets that a project selects.
    fn rule_set_ids_for(&self, project_id: &str) -> Result<Vec<String>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT p.rule_set_id FROM project_rule_sets p JOIN rule_sets r ON r.id = p.rule_set_id
             WHERE p.project_id = ?1 ORDER BY r.name COLLATE NOCASE",
        )?;
        let rows = st
            .query_map([project_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }

    pub fn set_project_rule_sets(&self, project_id: &str, ids: &[String]) -> Result<Project> {
        let known: Vec<String> = self.list_rule_sets()?.into_iter().map(|r| r.id).collect();
        for id in ids {
            if !known.contains(id) {
                bail!("No rule set with ID \"{id}\".");
            }
        }
        {
            let mut c = self.c();
            let tx = c.transaction()?;
            tx.execute("DELETE FROM project_rule_sets WHERE project_id = ?1", [project_id])?;
            for id in ids {
                tx.execute(
                    "INSERT OR IGNORE INTO project_rule_sets (project_id, rule_set_id) VALUES (?1, ?2)",
                    params![project_id, id],
                )?;
            }
            tx.commit()?;
        }
        self.find_project(project_id)
    }

    /// The rule sets that apply to a project: "always" sets first, then the selected ones.
    pub fn effective_rule_sets(&self, project: Option<&Project>) -> Result<Vec<RuleSet>> {
        let all = self.list_rule_sets()?;
        let mut out: Vec<RuleSet> = all.iter().filter(|r| r.always).cloned().collect();
        if let Some(p) = project {
            for r in all.into_iter().filter(|r| !r.always && p.rule_set_ids.contains(&r.id)) {
                out.push(r);
            }
        }
        Ok(out)
    }

    fn flag(&self, key: &str) -> Result<bool> {
        Ok(self
            .c()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get::<_, String>(0))
            .optional()?
            .is_some())
    }

    fn set_flag(&self, key: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO settings (key, value) VALUES (?1, '1') ON CONFLICT(key) DO NOTHING",
            [key],
        )?;
        Ok(())
    }

    /// The starter rule set: common FDM limits for PETG on a Bambu P1S.
    pub fn starter_rule_set() -> RuleSetInput {
        let v = |name: &str, value: f64, unit: &str, note: &str| Rule {
            name: name.into(),
            value: Some(value),
            unit: Some(unit.into()),
            note: note.into(),
        };
        let t = |name: &str, note: &str| Rule { name: name.into(), value: None, unit: None, note: note.into() };
        RuleSetInput {
            name: "FDM · PETG · Bambu P1S".into(),
            description: "Hard limits for parts printed in PETG on the Bambu P1S with a 0.4 mm nozzle and 0.2 mm layers. These are common starting values: check them against your Bambu Studio profiles and test prints, then edit them.".into(),
            always: false,
            rules: vec![
                v("Minimum wall", 1.2, "mm", "Three perimeters at a 0.4 mm line width."),
                v("Load-bearing wall", 2.0, "mm", "Walls that carry load or hold screws or heat-set inserts."),
                v("Minimum standing feature", 0.8, "mm", "Smallest rib, pin or lettering stroke."),
                v("Embossed or engraved detail", 0.6, "mm", "Minimum height or depth: three layers."),
                v("Maximum unsupported overhang", 45.0, "deg", "Measured from the vertical. Steeper faces need support or a redesign."),
                v("Maximum bridge", 10.0, "mm", "Longest unsupported horizontal span."),
                v("Layer height", 0.2, "mm", "Vertical detail finer than this does not print."),
                v("Sliding clearance", 0.3, "mm", "Gap on each side for parts that slide or turn."),
                v("Press-fit clearance", 0.1, "mm", "Gap on each side for press fits."),
                v("Vertical hole oversize", 0.2, "mm", "Add to the nominal diameter of holes printed vertically; holes print small."),
                v("Build-plate edge chamfer", 0.5, "mm", "Use a chamfer, not a fillet, on edges that touch the build plate."),
                v("Maximum part size", 250.0, "mm", "The P1S build volume is 256 × 256 × 256 mm. Split larger parts and join them with pins or dovetails."),
                t("Flat face on the build plate", "Each part needs one flat face that sits on the build plate, or a planned support strategy."),
                t("Load along the layers", "Layer lines are the weakest direction. Orient parts so that bending and tension loads run along the layers (X–Y), not across them (Z)."),
                t("Fillets up, chamfers down", "Fillet upward-facing and vertical edges freely. On downward-facing edges, a fillet makes an overhang; use a 45° chamfer."),
                t("Horizontal holes", "Give horizontal holes wider than the maximum bridge a teardrop or flat-top shape."),
                t("No support in closed cavities", "Support material inside a closed cavity cannot come out. Design cavities to print without support."),
            ],
        }
    }

    /// Version 0.3: process profiles become rule sets, and project process
    /// pins become rule set selections. Runs one time.
    fn migrate_rule_sets(&self) -> Result<()> {
        if self.flag("rule_sets_migrated")? {
            return Ok(());
        }
        #[derive(serde::Deserialize)]
        struct OldConstraint {
            name: String,
            value: f64,
            unit: String,
            #[serde(default)]
            note: String,
        }
        let rows: Vec<(String, String, String, String)> = {
            let c = self.c();
            let mut st = c.prepare("SELECT id, name, description, constraints FROM processes")?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let starter = Self::starter_rule_set();
        for (id, name, description, constraints) in rows {
            let old: Vec<OldConstraint> = serde_json::from_str(&constraints).unwrap_or_default();
            // The untouched 0.2 starter profile becomes the fuller 0.3 starter set.
            let untouched = name == "FDM · PETG · 0.4 mm nozzle"
                && old.len() == 9
                && old.iter().any(|c| c.name == "minimum wall" && (c.value - 1.2).abs() < 1e-9);
            let input = if untouched {
                starter.clone()
            } else {
                RuleSetInput {
                    name: name.clone(),
                    description,
                    always: false,
                    rules: old
                        .into_iter()
                        .map(|c| Rule { name: c.name, value: Some(c.value), unit: Some(c.unit), note: c.note })
                        .collect(),
                }
            };
            if self.get_rule_set(&id).is_err() {
                if let Err(e) = self.insert_rule_set(&id, &input) {
                    // A name clash: keep the old name with a suffix.
                    let mut renamed = input.clone();
                    renamed.name = format!("{} (from 0.2)", input.name);
                    self.insert_rule_set(&id, &renamed).map_err(|_| e)?;
                }
            }
        }
        self.c().execute(
            "INSERT OR IGNORE INTO project_rule_sets (project_id, rule_set_id)
             SELECT p.id, p.process_id FROM projects p
             WHERE p.process_id IS NOT NULL AND p.process_id IN (SELECT id FROM rule_sets)",
            [],
        )?;
        self.set_flag("rule_sets_migrated")
    }

    /// Adds the starter rule set one time, in a library without rule sets.
    fn seed_rule_sets(&self) -> Result<()> {
        if self.flag("rule_sets_seeded")? {
            return Ok(());
        }
        if self.list_rule_sets()?.is_empty() {
            self.create_rule_set(&Self::starter_rule_set())?;
        }
        self.set_flag("rule_sets_seeded")
    }

    // ------------------------------------------------------------------
    // Settings and usage
    // ------------------------------------------------------------------

    pub fn settings(&self) -> Result<Settings> {
        let c = self.c();
        let mut st = c.prepare("SELECT key, value FROM settings")?;
        let mut s = Settings::default();
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (k, v) in rows {
            match k.as_str() {
                "base_url" if !v.trim().is_empty() => s.base_url = v,
                "model" if !v.trim().is_empty() => s.model = v,
                "debounce_secs" => s.debounce_secs = v.parse().unwrap_or(s.debounce_secs),
                "monthly_token_budget" => {
                    s.monthly_token_budget = v.parse().unwrap_or(s.monthly_token_budget)
                }
                "thinking" if ["off", "low", "high"].contains(&v.as_str()) => s.thinking = v,
                _ => {}
            }
        }
        Ok(s)
    }

    pub fn save_settings(&self, s: &Settings) -> Result<Settings> {
        let c = self.c();
        let pairs = [
            ("base_url", s.base_url.trim().trim_end_matches('/').to_string()),
            ("model", s.model.trim().to_string()),
            ("debounce_secs", s.debounce_secs.clamp(5, 3600).to_string()),
            ("monthly_token_budget", s.monthly_token_budget.max(0).to_string()),
            ("thinking", {
                let t = s.thinking.trim().to_ascii_lowercase();
                if ["off", "low", "high"].contains(&t.as_str()) { t } else { "low".to_string() }
            }),
        ];
        for (k, v) in pairs {
            c.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
        }
        drop(c);
        self.settings()
    }

    pub fn usage(&self) -> Result<Usage> {
        let month = month_key();
        let row = self
            .c()
            .query_row(
                "SELECT input_tokens, output_tokens, requests FROM usage WHERE month = ?1",
                [&month],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (input_tokens, output_tokens, requests) = row.unwrap_or((0, 0, 0));
        Ok(Usage {
            month,
            input_tokens,
            output_tokens,
            requests,
        })
    }

    pub fn add_usage(&self, input: i64, output: i64) -> Result<()> {
        self.c().execute(
            "INSERT INTO usage (month, input_tokens, output_tokens, requests) VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(month) DO UPDATE SET input_tokens = input_tokens + ?2,
                 output_tokens = output_tokens + ?3, requests = requests + 1",
            params![month_key(), input, output],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Job queue
    // ------------------------------------------------------------------

    /// Adds a job, or moves an existing job of the same kind and target to the new time.
    pub fn enqueue(&self, kind: &str, target: &str, delay_secs: i64) -> Result<()> {
        let now = now_ts();
        self.c().execute(
            "INSERT INTO jobs (kind, target, run_after, attempts, gen, created_at) VALUES (?1, ?2, ?3, 0, 1, ?4)
             ON CONFLICT(kind, target) DO UPDATE SET run_after = excluded.run_after, attempts = 0,
                 last_error = NULL, gen = jobs.gen + 1",
            params![kind, target, now + delay_secs.max(0), now],
        )?;
        Ok(())
    }

    /// True when the language extends a base: its document comes from the
    /// base and the overrides, not from a synthesis pass.
    fn is_derived(&self, language_id: &str) -> Result<bool> {
        let base: Option<Option<String>> = self.c().query_row("SELECT base_id FROM languages WHERE id = ?1", [language_id], |r| r.get(0)).optional()?;
        Ok(base.flatten().is_some())
    }

    /// Queues the synthesis pass after the debounce time of the settings.
    pub fn enqueue_synth(&self, language_id: &str) -> Result<()> {
        if self.is_derived(language_id)? {
            return Ok(());
        }
        let delay = self.settings()?.debounce_secs;
        self.enqueue(JOB_SYNTH, language_id, delay)
    }

    /// Queues the synthesis pass now ("Synthesize now").
    pub fn synthesize_now(&self, language_id: &str) -> Result<()> {
        let lang = self.get_language(language_id)?;
        if let Some(base) = &lang.base_id {
            let name = self.get_language(base).map(|b| b.name).unwrap_or_else(|_| base.clone());
            bail!("\"{}\" extends \"{name}\": its document comes from that language and the overrides, not from a synthesis pass.", lang.name);
        }
        self.enqueue(JOB_SYNTH, language_id, 0)
    }

    /// The next job that is due. Item analysis runs before synthesis.
    pub fn next_due_job(&self) -> Result<Option<Job>> {
        self.next_job(false)
    }

    /// The next job. With `ignore_time`, waiting times (the synthesis wait) do
    /// not count: used when you start the queue by hand.
    pub fn next_job(&self, ignore_time: bool) -> Result<Option<Job>> {
        let limit = if ignore_time { i64::MAX } else { now_ts() };
        Ok(self
            .c()
            .query_row(
                "SELECT id, kind, target, run_after, attempts, gen FROM jobs WHERE run_after <= ?1
                 ORDER BY CASE kind WHEN 'synthesize' THEN 1 ELSE 0 END, run_after, id LIMIT 1",
                [limit],
                |r| {
                    Ok(Job {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        target: r.get(2)?,
                        run_after: r.get(3)?,
                        attempts: r.get(4)?,
                        gen: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    /// Removes a finished job, unless something queued it again while it ran.
    pub fn complete_job(&self, job: &Job) -> Result<()> {
        self.c().execute(
            "DELETE FROM jobs WHERE id = ?1 AND gen = ?2",
            params![job.id, job.gen],
        )?;
        Ok(())
    }

    pub fn drop_job(&self, job: &Job) -> Result<()> {
        self.c().execute("DELETE FROM jobs WHERE id = ?1", [job.id])?;
        Ok(())
    }

    /// Moves a job later without a count of a failed attempt.
    pub fn defer_job(&self, job: &Job, delay_secs: i64) -> Result<()> {
        self.c().execute(
            "UPDATE jobs SET run_after = ?3 WHERE id = ?1 AND gen = ?2",
            params![job.id, job.gen, now_ts() + delay_secs],
        )?;
        Ok(())
    }

    /// Records a failed attempt. Returns the new number of attempts.
    pub fn retry_job(&self, job: &Job, delay_secs: i64, error: &str) -> Result<i64> {
        let c = self.c();
        c.execute(
            "UPDATE jobs SET attempts = attempts + 1, run_after = ?3, last_error = ?4 WHERE id = ?1 AND gen = ?2",
            params![job.id, job.gen, now_ts() + delay_secs, error],
        )?;
        let attempts: Option<i64> = c
            .query_row("SELECT attempts FROM jobs WHERE id = ?1", [job.id], |r| r.get(0))
            .optional()?;
        Ok(attempts.unwrap_or(job.attempts + 1))
    }

    /// Every job in the queue, items first, with names for display.
    pub fn list_jobs(&self) -> Result<Vec<JobView>> {
        let names: std::collections::HashMap<String, String> =
            self.list_languages()?.into_iter().map(|l| (l.id, l.name)).collect();
        let c = self.c();
        let mut st = c.prepare(
            "SELECT j.id, j.kind, j.target, j.run_after, j.attempts, j.last_error,
                    CASE j.kind WHEN 'analyze_item' THEN i.language_id WHEN 'analyze_source' THEN s.language_id ELSE j.target END,
                    COALESCE(i.original_name, s.title)
             FROM jobs j LEFT JOIN items i ON (j.kind = 'analyze_item' AND i.id = j.target)
                         LEFT JOIN sources s ON (j.kind = 'analyze_source' AND s.id = j.target)
             ORDER BY CASE j.kind WHEN 'synthesize' THEN 1 ELSE 0 END, j.run_after, j.id",
        )?;
        let rows = st
            .query_map([], |r| {
                let lang: Option<String> = r.get(6)?;
                Ok(JobView {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    target: r.get(2)?,
                    run_after_ms: r.get::<_, i64>(3)? * 1000,
                    attempts: r.get(4)?,
                    last_error: r.get(5)?,
                    language_id: lang.clone().unwrap_or_default(),
                    language_name: String::new(),
                    item_name: r.get(7)?,
                    running: false,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|mut j| {
                j.language_name = names.get(&j.language_id).cloned().unwrap_or_else(|| j.language_id.clone());
                j
            })
            .collect())
    }

    /// After a cancel: an item keeps its last analysis if it has one;
    /// otherwise it shows as stopped, ready for "Analyze again".
    pub fn mark_item_cancelled(&self, item_id: &str) -> Result<()> {
        if self.get_analysis(item_id)?.is_some() {
            self.set_item_status(item_id, "done", None)
        } else {
            self.set_item_status(item_id, "error", Some("Cancelled by you. Click Analyze again to run it."))
        }
    }

    /// Removes one job from the queue. Returns the language it belonged to.
    pub fn cancel_job(&self, job_id: i64) -> Result<Option<String>> {
        let job: Option<(String, String)> = self
            .c()
            .query_row("SELECT kind, target FROM jobs WHERE id = ?1", [job_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((kind, target)) = job else { return Ok(None) };
        self.c().execute("DELETE FROM jobs WHERE id = ?1", [job_id])?;
        if kind == JOB_ANALYZE {
            let lang = self.get_item(&target).ok().map(|i| i.language_id);
            let _ = self.mark_item_cancelled(&target);
            return Ok(lang);
        }
        if kind == JOB_SOURCE {
            let lang = self.get_source(&target).ok().map(|s| s.language_id);
            let _ = self.mark_source_cancelled(&target);
            return Ok(lang);
        }
        Ok(Some(target))
    }

    /// Removes every job of a language (or of all languages). Returns the count.
    pub fn cancel_jobs(&self, language_id: Option<&str>) -> Result<usize> {
        let jobs = self.list_jobs()?;
        let mut n = 0;
        for j in jobs.iter().filter(|j| language_id.map(|l| j.language_id == l).unwrap_or(true)) {
            if self.cancel_job(j.id)?.is_some() {
                n += 1;
            }
        }
        Ok(n)
    }

    /// A paused queue starts no new jobs. The setting survives a restart.
    pub fn queue_paused(&self) -> Result<bool> {
        let v: Option<String> = self
            .c()
            .query_row("SELECT value FROM settings WHERE key = 'queue_paused'", [], |r| r.get(0))
            .optional()?;
        Ok(v.as_deref() == Some("1"))
    }

    pub fn set_queue_paused(&self, paused: bool) -> Result<()> {
        self.c().execute(
            "INSERT INTO settings (key, value) VALUES ('queue_paused', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [if paused { "1" } else { "0" }],
        )?;
        // A pause or a resume by hand ends the reason of an earlier pause by the app.
        self.c().execute("DELETE FROM settings WHERE key = 'queue_pause_reason'", [])?;
        Ok(())
    }

    /// Manual mode: jobs collect in the queue until you start it.
    /// Pauses the queue because no job can run: "plan_limit" (the ChatGPT plan has no
    /// usage left) or "sign_in" (the ChatGPT sign-in ended). The app shows the reason.
    pub fn pause_queue_for(&self, reason: &str) -> Result<()> {
        self.set_queue_paused(true)?;
        self.c().execute(
            "INSERT INTO settings (key, value) VALUES ('queue_pause_reason', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [reason],
        )?;
        Ok(())
    }

    /// Why the app paused the queue. None when the person paused it, or when it runs.
    pub fn queue_pause_reason(&self) -> Result<Option<String>> {
        Ok(self
            .c()
            .query_row("SELECT value FROM settings WHERE key = 'queue_pause_reason'", [], |r| r.get(0))
            .optional()?)
    }

    pub fn queue_manual(&self) -> Result<bool> {
        let v: Option<String> = self
            .c()
            .query_row("SELECT value FROM settings WHERE key = 'queue_manual'", [], |r| r.get(0))
            .optional()?;
        Ok(v.as_deref() == Some("1"))
    }

    pub fn set_queue_manual(&self, manual: bool) -> Result<()> {
        self.c().execute(
            "INSERT INTO settings (key, value) VALUES ('queue_manual', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [if manual { "1" } else { "0" }],
        )?;
        Ok(())
    }

    pub fn job_count(&self) -> Result<i64> {
        Ok(self.c().query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))?)
    }

    pub fn pending_analyze_count(&self, language_id: &str) -> Result<i64> {
        Ok(self.c().query_row(
            PENDING_SQL,
            [language_id],
            |r| r.get(0),
        )?)
    }

    pub fn language_statuses(&self) -> Result<Vec<LanguageStatus>> {
        let now = now_ts();
        let langs = self.list_languages()?;
        let c = self.c();
        let mut out = Vec::new();
        for l in langs {
            let pending_items: i64 = c.query_row(
                PENDING_SQL,
                [&l.id],
                |r| r.get(0),
            )?;
            let error_items: i64 = c.query_row(
                "SELECT (SELECT COUNT(*) FROM items WHERE language_id = ?1 AND analysis_status = 'error')
                      + (SELECT COUNT(*) FROM sources WHERE language_id = ?1 AND analysis_status = 'error')",
                [&l.id],
                |r| r.get(0),
            )?;
            let synth: Option<i64> = c
                .query_row(
                    "SELECT run_after FROM jobs WHERE kind = 'synthesize' AND target = ?1",
                    [&l.id],
                    |r| r.get(0),
                )
                .optional()?;
            let open_proposals: i64 = c.query_row(
                "SELECT COUNT(*) FROM proposals WHERE language_id = ?1 AND status = 'pending'",
                [&l.id],
                |r| r.get(0),
            )?;
            let pending_outputs: i64 = c.query_row(
                "SELECT COUNT(*) FROM outputs WHERE language_id = ?1 AND verdict = 'pending'",
                [&l.id],
                |r| r.get(0),
            )?;
            out.push(LanguageStatus {
                language_id: l.id,
                pending_items,
                error_items,
                synth_due_in: synth.map(|t| (t - now).max(0)),
                open_proposals,
                pending_outputs,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_open_preserves_an_older_library() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)").unwrap();
        drop(db);
        let original = std::fs::read(&path).unwrap();

        let lib = Library::open_read_only(dir.path()).unwrap();
        assert!(!lib.inventory_enabled().unwrap());
        assert!(lib.set_inventory_enabled(true).is_err());
        drop(lib);

        assert_eq!(std::fs::read(path).unwrap(), original);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn read_only_open_does_not_create_a_missing_library() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Library::open_read_only(dir.path()).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn read_only_open_sees_committed_app_edits_in_wal() {
        let dir = tempfile::tempdir().unwrap();
        let writer = Library::open(dir.path()).unwrap();
        let reader = Library::open_read_only(dir.path()).unwrap();
        assert!(!reader.inventory_enabled().unwrap());
        writer.set_inventory_enabled(true).unwrap();
        assert!(reader.inventory_enabled().unwrap());
        assert!(reader.set_inventory_enabled(false).is_err());
        assert!(writer.inventory_enabled().unwrap());
    }

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, 120]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    #[test]
    fn full_flow() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Quiet Instrument", "Soft matte objects").unwrap();
        assert_eq!(lang.id, "quiet-instrument");
        let lang2 = lib.create_language("Quiet Instrument", "").unwrap();
        assert_eq!(lang2.id, "quiet-instrument-2");

        let bytes = png_bytes(300, 200);
        let item = lib.import_bytes(&lang.id, "a.png", &bytes, Some("https://x.test/a.png")).unwrap();
        assert_eq!(item.width, 300);
        assert!(lib.abs(&item.thumb_rel).unwrap().exists());
        // Same bytes again: no duplicate.
        let again = lib.import_bytes(&lang.id, "a.png", &bytes, None).unwrap();
        assert_eq!(again.id, item.id);
        assert_eq!(lib.list_items(&lang.id).unwrap().len(), 1);

        // Video bytes are not an image: the message points to the video import.
        let err = lib.import_bytes(&lang.id, "clip.mov", b"not an image", None).unwrap_err();
        assert!(err.to_string().contains("key frames"), "{err}");
        // A video that macOS cannot read gets a clear message too.
        let mkv = dir.path().join("clip.mkv");
        std::fs::write(&mkv, b"x").unwrap();
        let err = lib.import_video(&lang.id, &mkv, "clip.mkv", None).unwrap_err();
        assert!(err.to_string().contains("Convert"), "{err}");

        // Annotation change queues analysis.
        let updated = lib
            .update_item(&item.id, &["Edge".into(), "edge".into(), " grip ".into()], "Note", 9, true)
            .unwrap();
        assert_eq!(updated.tags, vec!["edge".to_string(), "grip".to_string()]);
        assert_eq!(updated.weight, 5);
        assert!(updated.anti);

        let regions = lib
            .set_regions(
                &item.id,
                &[RegionInput { id: None, x: 0.1, y: 0.1, w: 0.3, h: 0.3, note: "fillet".into(), tag: "Edge".into() }],
            )
            .unwrap();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].tag, "edge");

        // The analyze job is due now (import queued it at delay 0, then later moved it).
        lib.enqueue(JOB_ANALYZE, &item.id, 0).unwrap();
        let job = lib.next_due_job().unwrap().unwrap();
        assert_eq!(job.kind, JOB_ANALYZE);
        lib.save_analysis(&item.id, &serde_json::json!({"summary": "test"}), "m").unwrap();
        lib.complete_job(&job).unwrap();
        assert_eq!(lib.pending_analyze_count(&lang.id).unwrap(), 0);

        let inputs = lib.synthesis_inputs(&lang.id, 10).unwrap();
        assert_eq!(inputs.len(), 1);

        let tokens = serde_json::json!({
            "geometry": {}, "cmf": [], "prompts": {"positive": "a", "negative": "b"}, "do": [], "dont": []
        });
        let v = lib.save_version(&lang.id, "# Doc", &tokens, "m", &[item.id.clone()], false, "h1").unwrap();
        assert_eq!(v, 1);
        let content = lib.read_version(&lang.id, 1).unwrap();
        assert_eq!(content.tokens["version"], 1);
        let v2 = lib.save_edited_version(&lang.id, "# Doc 2", &tokens.to_string()).unwrap();
        assert_eq!(v2, 2);
        assert!(lib.save_edited_version(&lang.id, "x", "{\"geometry\": 1}").is_err());

        let p = lib.create_project("Lamp").unwrap();
        assert!(lib.create_project("lamp").is_err());
        let p = lib
            .set_pins(&p.id, &[PinInput { language_id: lang.id.clone(), version: 1, role: "primary".into(), scope: "all".into() }])
            .unwrap();
        assert_eq!(p.pins.len(), 1);
        assert_eq!(p.pins[0].latest_version, Some(2));
        assert!(lib
            .set_pins(&p.id, &[PinInput { language_id: lang.id.clone(), version: 9, role: "primary".into(), scope: "all".into() }])
            .is_err());

        lib.remove_item(&item.id).unwrap();
        assert!(lib.list_items(&lang.id).unwrap().is_empty());
        assert!(lib.lang_dir(&lang.id).join("_removed").read_dir().unwrap().count() >= 1);

        let mut s = lib.settings().unwrap();
        assert_eq!(s.model, "deepseek-flash");
        s.debounce_secs = 1;
        let s = lib.save_settings(&s).unwrap();
        assert_eq!(s.debounce_secs, 5);
        lib.add_usage(10, 5).unwrap();
        assert_eq!(lib.usage().unwrap().input_tokens, 10);
    }

    #[test]
    fn migrates_v01_library() {
        let dir = tempfile::tempdir().unwrap();
        {
            // A database as version 0.1 made it: no schema/warnings/process_id columns.
            let c = rusqlite::Connection::open(dir.path().join("library.sqlite")).unwrap();
            c.execute_batch(
                "CREATE TABLE languages (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE versions (language_id TEXT NOT NULL, version INTEGER NOT NULL, created_at TEXT NOT NULL, model TEXT NOT NULL, item_ids TEXT NOT NULL DEFAULT '[]', edited INTEGER NOT NULL DEFAULT 0, input_hash TEXT NOT NULL DEFAULT '', PRIMARY KEY (language_id, version));
                 CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE, created_at TEXT NOT NULL);
                 INSERT INTO languages VALUES ('organic', 'Organic', '', '2026-11-06T00:00:00Z', '2026-11-06T00:00:00Z');
                 INSERT INTO versions VALUES ('organic', 1, '2026-11-06T00:00:00Z', 'deepseek-flash', '[]', 0, 'h');
                 INSERT INTO projects VALUES ('p1', 'Bench', '2026-11-06T00:00:00Z');",
            )
            .unwrap();
        }
        let lib = Library::open(dir.path()).unwrap();
        let v = lib.list_versions("organic").unwrap();
        assert_eq!(v[0].schema, 1);
        assert!(v[0].warnings.is_empty());
        let projects = lib.list_projects().unwrap();
        assert!(projects[0].rule_set_ids.is_empty());
        // The starter rule set exists one time, also after a second open.
        assert_eq!(lib.list_rule_sets().unwrap().len(), 1);
        drop(lib);
        let lib = Library::open(dir.path()).unwrap();
        assert_eq!(lib.list_rule_sets().unwrap().len(), 1);
    }

    #[test]
    fn migrates_v02_process_profiles() {
        let dir = tempfile::tempdir().unwrap();
        {
            let c = crate::db::open(&dir.path().join("library.sqlite")).unwrap();
            let seed = r#"[{"name":"minimum wall","value":1.2,"unit":"mm","note":""},{"name":"b","value":1,"unit":"mm"},{"name":"c","value":1,"unit":"mm"},{"name":"d","value":1,"unit":"mm"},{"name":"e","value":1,"unit":"mm"},{"name":"f","value":1,"unit":"mm"},{"name":"g","value":1,"unit":"mm"},{"name":"h","value":1,"unit":"mm"},{"name":"i","value":1,"unit":"mm"}]"#;
            c.execute("INSERT INTO processes VALUES ('pr1', 'FDM · PETG · 0.4 mm nozzle', 'old', ?1, 't', 't')", [seed]).unwrap();
            c.execute("INSERT INTO processes VALUES ('pr2', 'CNC · 6061', 'cnc', '[{\"name\":\"minimum wall\",\"value\":1.5,\"unit\":\"mm\",\"note\":\"x\"}]', 't', 't')", []).unwrap();
            c.execute("INSERT INTO settings VALUES ('processes_seeded', '1')", []).unwrap();
            c.execute("INSERT INTO projects (id, name, created_at, process_id) VALUES ('p1', 'Bench', 't', 'pr2')", []).unwrap();
        }
        let lib = Library::open(dir.path()).unwrap();
        let sets = lib.list_rule_sets().unwrap();
        assert_eq!(sets.len(), 2);
        let petg = lib.get_rule_set("pr1").unwrap();
        assert_eq!(petg.name, "FDM · PETG · Bambu P1S"); // untouched starter upgraded
        assert!(petg.rules.iter().any(|r| r.value.is_none()));
        let cnc = lib.get_rule_set("CNC · 6061").unwrap();
        assert_eq!(cnc.rules[0], Rule { name: "minimum wall".into(), value: Some(1.5), unit: Some("mm".into()), note: "x".into() });
        assert_eq!(cnc.project_count, 1);
        assert_eq!(lib.find_project("bench").unwrap().rule_set_ids, vec!["pr2".to_string()]);
        drop(lib);
        let lib = Library::open(dir.path()).unwrap();
        assert_eq!(lib.list_rule_sets().unwrap().len(), 2);
    }

    #[test]
    fn rule_sets_and_spec_versions() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let petg = lib.list_rule_sets().unwrap().remove(0);
        assert!(petg.name.contains("PETG"));
        assert!(petg.rules.iter().any(|r| r.name == "Minimum wall" && r.value == Some(1.2)));

        let input = RuleSetInput {
            name: "Safety · children".into(),
            description: "".into(),
            always: true,
            rules: vec![
                Rule { name: "Smallest loose part".into(), value: Some(32.0), unit: Some("mm".into()), note: "".into() },
                Rule { name: "No sharp edges".into(), value: None, unit: Some("mm".into()), note: "Round every exposed edge.".into() },
                Rule { name: "".into(), value: None, unit: None, note: "".into() },
            ],
        };
        let safety = lib.create_rule_set(&input).unwrap();
        assert_eq!(safety.rules.len(), 2); // the empty row is dropped
        assert_eq!(safety.rules[1].unit, None); // no unit without a value
        assert!(lib.create_rule_set(&input).is_err()); // same name
        let bad = RuleSetInput { name: "x".into(), description: "".into(), always: false, rules: vec![Rule { name: "a".into(), value: Some(1.0), unit: Some("inch".into()), note: "".into() }] };
        assert!(lib.create_rule_set(&bad).is_err());

        let p = lib.create_project("Bench").unwrap();
        let p = lib.set_project_rule_sets(&p.id, &[petg.id.clone()]).unwrap();
        assert_eq!(p.rule_set_ids, vec![petg.id.clone()]);
        let eff = lib.effective_rule_sets(Some(&p)).unwrap();
        assert_eq!(eff.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Safety · children", "FDM · PETG · Bambu P1S"]);
        assert_eq!(lib.effective_rule_sets(None).unwrap().len(), 1);
        assert!(lib.set_project_rule_sets(&p.id, &["nope".into()]).is_err());
        lib.delete_rule_set(&petg.id).unwrap();
        assert!(lib.find_project(&p.id).unwrap().rule_set_ids.is_empty());

        // Schema 2 version: the app renders the markdown and keeps the warnings.
        let lang = lib.create_language("Organic", "").unwrap();
        let spec_value = crate::spec::tests::sample();
        let v = lib
            .save_spec_version(&lang.id, &spec_value, "m", &[], false, "h", &["check this".into()])
            .unwrap();
        let c = lib.read_version(&lang.id, v).unwrap();
        assert_eq!(c.info.schema, 2);
        assert_eq!(c.info.warnings, vec!["check this".to_string()]);
        assert_eq!(c.tokens["schema"], 2);
        assert!(c.markdown.contains("## Object families"));

        // An edit of a schema 2 spec validates it and renders new markdown.
        let mut edited = c.tokens.clone();
        edited["summary"] = serde_json::json!("A new summary.");
        let v2 = lib.save_edited_version(&lang.id, "ignored", &edited.to_string()).unwrap();
        let c2 = lib.read_version(&lang.id, v2).unwrap();
        assert!(c2.info.edited);
        assert!(c2.markdown.contains("A new summary."));
        let mut broken = c.tokens.clone();
        broken["families"] = serde_json::json!([]);
        assert!(lib.save_edited_version(&lang.id, "", &broken.to_string()).is_err());
    }

    #[test]
    fn queue_cancel_and_pause() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Organic", "").unwrap();
        let a = lib.import_bytes(&lang.id, "a.png", &png_bytes(64, 48), None).unwrap();
        let b = lib.import_bytes(&lang.id, "b.png", &png_bytes(48, 64), None).unwrap();
        let jobs = lib.list_jobs().unwrap();
        assert_eq!(jobs.len(), 3); // two analyses + one synthesis
        assert_eq!(jobs[0].language_name, "Organic");
        assert!(jobs.iter().any(|j| j.item_name.as_deref() == Some("a.png")));
        // Cancel one analysis: the item without an analysis shows as stopped.
        let ja = jobs.iter().find(|j| j.target == a.id).unwrap().id;
        assert_eq!(lib.cancel_job(ja).unwrap().as_deref(), Some(lang.id.as_str()));
        let ia = lib.get_item(&a.id).unwrap();
        assert_eq!(ia.analysis_status, "error");
        assert!(ia.analysis_error.unwrap().contains("Cancelled"));
        // An item with an earlier analysis keeps "done".
        lib.save_analysis(&b.id, &serde_json::json!({"summary": "x"}), "m").unwrap();
        lib.set_item_status(&b.id, "pending", None).unwrap();
        assert_eq!(lib.cancel_jobs(Some(&lang.id)).unwrap(), 2);
        assert_eq!(lib.get_item(&b.id).unwrap().analysis_status, "done");
        assert_eq!(lib.job_count().unwrap(), 0);
        assert!(lib.cancel_job(9999).unwrap().is_none());
        assert!(!lib.queue_paused().unwrap());
        lib.set_queue_paused(true).unwrap();
        assert!(lib.queue_paused().unwrap());
    }

    #[test]
    fn sources_flow() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Organic", "").unwrap();
        let text = b"# Fillet guide\n\nUse a fillet radius of about 0.4 times the wall thickness on hand edges.";
        let doc = lib.import_document(&lang.id, "fillet-guide.md", text).unwrap();
        assert_eq!(doc.kind, "document");
        assert_eq!(doc.media, "text");
        assert!(lib.abs(doc.file_rel.as_deref().unwrap()).unwrap().exists());
        assert_eq!(lib.import_document(&lang.id, "copy.md", text).unwrap().id, doc.id); // same bytes
        assert!(lib.source_text(&doc.id).unwrap().contains("0.4 times"));
        let ex = crate::sources::Extracted { title: "Video".into(), media: "video".into(), text: "Video title: x".into(), ..Default::default() };
        let link = lib.add_link_source(&lang.id, "https://youtu.be/abc", &ex).unwrap();
        assert_eq!(lib.add_link_source(&lang.id, "https://youtu.be/abc", &ex).unwrap().id, link.id);
        assert_eq!(lib.list_sources(&lang.id).unwrap().len(), 2);
        assert_eq!(lib.pending_analyze_count(&lang.id).unwrap(), 2);
        let jobs = lib.list_jobs().unwrap();
        assert!(jobs.iter().any(|j| j.kind == JOB_SOURCE && j.item_name.as_deref() == Some("Fillet guide") && j.language_name == "Organic"));
        lib.save_source_analysis(&doc.id, &serde_json::json!({"summary": "s"}), "m").unwrap();
        assert_eq!(lib.source_inputs(&lang.id, 10).unwrap().len(), 1);
        let jl = jobs.iter().find(|j| j.target == link.id).unwrap().id;
        lib.cancel_job(jl).unwrap();
        assert_eq!(lib.get_source(&link.id).unwrap().analysis_status, "error");
        assert_eq!(lib.retry_failed_items(&lang.id).unwrap(), 1);
        let s = lib.update_source(&doc.id, "Fillet guide", "Focus on edges", 5).unwrap();
        assert_eq!((s.title.as_str(), s.weight), ("Fillet guide", 5));
        assert_eq!(lib.get_source(&doc.id).unwrap().analysis_status, "pending");
        lib.remove_source(&doc.id).unwrap();
        assert_eq!(lib.list_sources(&lang.id).unwrap().len(), 1);
        assert_eq!(lib.reanalyze_all(&lang.id).unwrap(), 1);
    }

    #[test]
    fn canvas_places() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let a_lang = lib.create_language("A", "").unwrap();
        let b_lang = lib.create_language("B", "").unwrap();
        let a = lib.import_bytes(&a_lang.id, "a.png", &png_bytes(40, 30), None).unwrap();
        let b = lib.import_bytes(&b_lang.id, "b.png", &png_bytes(30, 40), None).unwrap();
        assert!(a.canvas.is_none());
        let jobs = lib.job_count().unwrap();

        let place = |id: &str, x: f64| CanvasPlacement { id: id.into(), x, y: -20.0, w: 300.0 };
        // Only items of the language move; another language's item and unknown IDs are skipped.
        let n = lib.set_canvas(&a_lang.id, &[place(&a.id, 10.0), place(&b.id, 99.0), place("nope", 1.0)]).unwrap();
        assert_eq!(n, 1);
        assert_eq!(lib.get_item(&a.id).unwrap().canvas, Some(CanvasPos { x: 10.0, y: -20.0, w: 300.0 }));
        assert!(lib.get_item(&b.id).unwrap().canvas.is_none());
        // Layout is not a reference change: no new jobs.
        assert_eq!(lib.job_count().unwrap(), jobs);
        // Widths are clamped; bad numbers are refused.
        lib.set_canvas(&a_lang.id, &[CanvasPlacement { id: a.id.clone(), x: 0.0, y: 0.0, w: 1.0 }]).unwrap();
        assert_eq!(lib.get_item(&a.id).unwrap().canvas.unwrap().w, 40.0);
        assert!(lib.set_canvas(&a_lang.id, &[CanvasPlacement { id: a.id.clone(), x: f64::NAN, y: 0.0, w: 100.0 }]).is_err());
        lib.clear_canvas(&a_lang.id).unwrap();
        assert!(lib.list_items(&a_lang.id).unwrap()[0].canvas.is_none());
    }
}
