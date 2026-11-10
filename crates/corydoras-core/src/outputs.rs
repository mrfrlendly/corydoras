//! Outputs: designs that an agent made from a language, sent back through the
//! MCP server (images only). The designer marks each one accepted or rejected
//! in the app.
//!
//! A decided output becomes an item of the language, so the normal pipeline
//! (analysis, synthesis) uses it:
//! - rejected: an anti-reference ("not this"), with the designer's note;
//! - accepted: a reference of low weight, tagged "agent-output". It supports
//!   the language but weighs less than the designer's own references.
//!
//! An undecided output is only stored. The MCP server can submit outputs on its
//! query-only connection (like proposals); deciding happens in the app.

use crate::analysis;
use crate::conformance::{Measurement, Report};
use crate::geometry::GeometryReport;
use crate::library::Library;
use crate::util::{fnv1a64, now_iso};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;
use std::path::Path;

/// Largest output image.
pub const MAX_OUTPUT_BYTES: usize = 20 * 1024 * 1024;
/// Most undecided outputs for one language.
pub const MAX_PENDING: i64 = 50;
/// Weight of an accepted output as a reference (the designer's own are 3 by default).
pub const ACCEPTED_WEIGHT: i64 = 2;
/// Weight of a rejected output as an anti-reference.
pub const REJECTED_WEIGHT: i64 = 3;

const COLS: &str = "id, language_id, project, name, file_name, width, height, language_version, author, note, \
    family, measurements, check_json, verdict, verdict_note, item_id, created_at, decided_at, geometry_json";

#[derive(Serialize, Clone, Debug)]
pub struct Output {
    pub id: String,
    pub language_id: String,
    pub project: String,
    pub name: String,
    /// Path of the image, relative to the library root.
    pub file_rel: String,
    /// Path of the thumbnail, relative to the library root.
    pub thumb_rel: String,
    pub width: i64,
    pub height: i64,
    /// The version of the language that the agent worked from.
    pub language_version: Option<i64>,
    pub author: String,
    pub note: String,
    pub family: String,
    pub measurements: Vec<Measurement>,
    pub check: Option<Report>,
    /// The geometry check of a model of this design, if one ran.
    pub geometry: Option<GeometryReport>,
    /// "pending", "accepted" or "rejected"
    pub verdict: String,
    pub verdict_note: String,
    /// The item that a decided output became.
    pub item_id: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

fn from_row(r: &Row) -> rusqlite::Result<Output> {
    let id: String = r.get(0)?;
    let language_id: String = r.get(1)?;
    let file_name: String = r.get(4)?;
    let measurements: String = r.get(11)?;
    let check: Option<String> = r.get(12)?;
    let geometry: Option<String> = r.get(18)?;
    Ok(Output {
        geometry: geometry.and_then(|g| serde_json::from_str(&g).ok()),
        file_rel: format!("languages/{language_id}/outputs/{file_name}"),
        thumb_rel: format!("languages/{language_id}/outputs/{id}.thumb.jpg"),
        project: r.get(2)?,
        name: r.get(3)?,
        width: r.get(5)?,
        height: r.get(6)?,
        language_version: r.get(7)?,
        author: r.get(8)?,
        note: r.get(9)?,
        family: r.get(10)?,
        measurements: serde_json::from_str(&measurements).unwrap_or_default(),
        check: check.and_then(|c| serde_json::from_str(&c).ok()),
        verdict: r.get(13)?,
        verdict_note: r.get(14)?,
        item_id: r.get(15)?,
        created_at: r.get(16)?,
        decided_at: r.get(17)?,
        id,
        language_id,
    })
}

impl Library {
    /// Runs a write on a connection that may be query-only (the MCP server).
    pub(crate) fn with_writes<T>(&self, f: impl FnOnce(&rusqlite::Connection) -> Result<T>) -> Result<T> {
        let c = self.c();
        let query_only: i64 = c.query_row("PRAGMA query_only", [], |r| r.get(0))?;
        c.execute_batch("PRAGMA query_only=OFF;")?;
        let res = f(&c);
        if query_only != 0 {
            c.execute_batch("PRAGMA query_only=ON;")?;
        }
        res
    }

    pub fn get_output(&self, id: &str) -> Result<Output> {
        self.c()
            .query_row(&format!("SELECT {COLS} FROM outputs WHERE id = ?1"), [id], from_row)
            .optional()?
            .ok_or_else(|| anyhow!("No output with the ID {id}."))
    }

    /// An output by its ID or by the first characters of it.
    pub fn find_output(&self, key: &str) -> Result<Output> {
        let key = key.trim();
        if key.len() < 6 {
            bail!("Give at least 6 characters of the output ID.");
        }
        let found: Vec<String> = {
            let c = self.c();
            let mut st = c.prepare("SELECT id FROM outputs WHERE id LIKE ?1 || '%' LIMIT 2")?;
            let rows = st.query_map([key], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            rows
        };
        match found.len() {
            0 => bail!("No output with the ID {key}."),
            1 => self.get_output(&found[0]),
            _ => bail!("More than one output starts with {key}. Give more characters."),
        }
    }

    /// Outputs of a language, newest first.
    pub fn list_outputs(&self, language_id: &str) -> Result<Vec<Output>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT {COLS} FROM outputs WHERE language_id = ?1 ORDER BY created_at DESC, rowid DESC"))?;
        let rows = st.query_map([language_id], from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn pending_output_count(&self, language_id: &str) -> Result<i64> {
        Ok(self.c().query_row(
            "SELECT COUNT(*) FROM outputs WHERE language_id = ?1 AND verdict = 'pending'",
            [language_id],
            |r| r.get(0),
        )?)
    }

    /// Stores an output image. Works on a query-only connection (the MCP server).
    /// The output waits for the designer's verdict; nothing else changes.
    pub fn submit_output(
        &self,
        language_key: &str,
        name: &str,
        bytes: &[u8],
        family: &str,
        project: &str,
        author: &str,
        note: &str,
        measurements: &[Measurement],
    ) -> Result<Output> {
        let lang = self.find_language(language_key)?;
        if bytes.is_empty() {
            bail!("The image is empty.");
        }
        if bytes.len() > MAX_OUTPUT_BYTES {
            bail!("The image is larger than 20 MB. Send a smaller render.");
        }
        let format = image::guess_format(bytes).map_err(|_| anyhow!("The data is not a PNG, JPEG, WebP or other image that Corydoras can read."))?;
        let img = analysis::decode_oriented(bytes).context("Cannot read the image")?;
        let pending = self.pending_output_count(&lang.id)?;
        if pending >= MAX_PENDING {
            bail!("\"{}\" has {pending} outputs without a verdict. The designer must decide on them before new ones can come in.", lang.name);
        }
        let hash = fnv1a64(bytes);
        let duplicate: Option<String> = self
            .c()
            .query_row("SELECT id FROM outputs WHERE language_id = ?1 AND hash = ?2", params![lang.id, hash], |r| r.get(0))
            .optional()?;
        if let Some(id) = duplicate {
            return self.get_output(&id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let ext = format.extensions_str().first().copied().unwrap_or("img");
        let file_name = format!("{id}.{ext}");
        let dir = self.lang_dir(&lang.id).join("outputs");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(&file_name), bytes)?;
        std::fs::write(dir.join(format!("{id}.thumb.jpg")), analysis::encode_jpeg(&img, 640, 84)?)?;
        let name = {
            let n = name.trim();
            if n.is_empty() { format!("output.{ext}") } else { n.chars().take(120).collect::<String>() }
        };
        let version = self.latest_version(&lang.id)?;
        let measurements_json = serde_json::to_string(measurements)?;
        let res = self.with_writes(|c| {
            c.execute(
                "INSERT INTO outputs (id, language_id, project, name, file_name, width, height, hash, language_version, author, note, family, measurements, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    id,
                    lang.id,
                    project.trim(),
                    name,
                    file_name,
                    img.width() as i64,
                    img.height() as i64,
                    hash,
                    version,
                    author.trim(),
                    note.trim(),
                    family.trim(),
                    measurements_json,
                    now_iso()
                ],
            )?;
            Ok(())
        });
        if let Err(e) = res {
            let _ = std::fs::remove_file(dir.join(&file_name));
            let _ = std::fs::remove_file(dir.join(format!("{id}.thumb.jpg")));
            return Err(e);
        }
        self.get_output(&id)
    }

    /// Saves a conformance report with an output (works on a query-only connection).
    pub fn save_output_check(&self, output_id: &str, report: &Report) -> Result<()> {
        let json = serde_json::to_string(report)?;
        self.with_writes(|c| {
            let n = c.execute("UPDATE outputs SET check_json = ?2 WHERE id = ?1", params![output_id, json])?;
            if n == 0 {
                bail!("No output with the ID {output_id}.");
            }
            Ok(())
        })
    }

    /// Saves a geometry report with an output (works on a query-only connection).
    pub fn save_output_geometry(&self, output_id: &str, report: &GeometryReport) -> Result<()> {
        let json = serde_json::to_string(report)?;
        self.with_writes(|c| {
            let n = c.execute("UPDATE outputs SET geometry_json = ?2 WHERE id = ?1", params![output_id, json])?;
            if n == 0 {
                bail!("No output with the ID {output_id}.");
            }
            Ok(())
        })
    }

    /// Marks an output accepted, rejected or pending again. A decided output
    /// becomes an item of the language; "pending" removes that item. Needs a
    /// writable connection (the app).
    pub fn set_output_verdict(&self, id: &str, verdict: &str, note: &str) -> Result<Output> {
        let out = self.get_output(id)?;
        let note = note.trim();
        match verdict {
            "pending" => {
                if let Some(item) = &out.item_id {
                    if self.get_item(item).is_ok() {
                        self.remove_item(item)?;
                    }
                }
                self.c().execute(
                    "UPDATE outputs SET verdict = 'pending', verdict_note = '', item_id = NULL, decided_at = NULL WHERE id = ?1",
                    [id],
                )?;
            }
            "accepted" | "rejected" => {
                let rejected = verdict == "rejected";
                let existing = out.item_id.as_deref().filter(|i| self.get_item(i).is_ok()).map(|s| s.to_string());
                let item_id = match existing {
                    Some(i) => i,
                    None => {
                        let bytes = std::fs::read(self.abs(&out.file_rel)?).context("The output image is missing from the library")?;
                        let source = format!("agent output {}", out.id);
                        let item = self.import_bytes(&out.language_id, &out.name, &bytes, Some(&source))?;
                        if item.source.as_deref() != Some(source.as_str()) {
                            bail!("This image is already a reference of the language, so it cannot also be an output.");
                        }
                        item.id
                    }
                };
                let item_note = if rejected {
                    format!(
                        "Made by an agent from this language{} and rejected by the designer{}",
                        out.language_version.map(|v| format!(" (v{v})")).unwrap_or_default(),
                        if note.is_empty() { ": not this.".to_string() } else { format!(": {note}") }
                    )
                } else {
                    format!(
                        "Made by an agent from this language{} and accepted by the designer. Supporting evidence, not a source of new rules.{}",
                        out.language_version.map(|v| format!(" (v{v})")).unwrap_or_default(),
                        if note.is_empty() { String::new() } else { format!(" {note}") }
                    )
                };
                let mut tags = vec!["agent-output".to_string()];
                if !out.family.trim().is_empty() {
                    tags.push(out.family.trim().to_string());
                }
                self.update_item(&item_id, &tags, &item_note, if rejected { REJECTED_WEIGHT } else { ACCEPTED_WEIGHT }, rejected)?;
                self.c().execute(
                    "UPDATE outputs SET verdict = ?2, verdict_note = ?3, item_id = ?4, decided_at = ?5 WHERE id = ?1",
                    params![id, verdict, note, item_id, now_iso()],
                )?;
            }
            other => bail!("Unknown verdict \"{other}\". Use accepted, rejected or pending."),
        }
        self.get_output(id)
    }

    /// Removes an output, its files and the item that it became.
    pub fn remove_output(&self, id: &str) -> Result<()> {
        let out = self.get_output(id)?;
        if let Some(item) = &out.item_id {
            if self.get_item(item).is_ok() {
                self.remove_item(item)?;
            }
        }
        for rel in [&out.file_rel, &out.thumb_rel] {
            let p = self.abs(rel)?;
            if p.exists() {
                let _ = std::fs::remove_file(Path::new(&p));
            }
        }
        self.c().execute("DELETE FROM outputs WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(seed: u8) -> Vec<u8> {
        let img = image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([(x as u8).wrapping_add(seed), y as u8, 90]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn output_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Quiet", "").unwrap();
        let ms = [Measurement { name: "minimum wall".into(), value: 1.4, unit: Some("mm".into()) }];

        // Submitting works on a query-only connection and changes no items.
        let ro = Library::open_query_only(dir.path()).unwrap();
        let out = ro.submit_output("quiet", "shade.png", &png(1), "shell", "lamp", "claude-code", "first try", &ms).unwrap();
        assert_eq!(out.verdict, "pending");
        assert_eq!(out.measurements.len(), 1);
        assert!(lib.abs(&out.file_rel).unwrap().exists());
        assert!(lib.abs(&out.thumb_rel).unwrap().exists());
        assert!(lib.list_items(&lang.id).unwrap().is_empty());
        // The same image again returns the same output.
        assert_eq!(ro.submit_output("quiet", "again.png", &png(1), "", "", "", "", &[]).unwrap().id, out.id);
        assert!(ro.submit_output("quiet", "x", b"not an image", "", "", "", "", &[]).is_err());
        // A read-only connection cannot decide.
        assert!(ro.set_output_verdict(&out.id, "rejected", "").is_err());

        // Rejected: an anti-reference with the note.
        let r = lib.set_output_verdict(&out.id, "rejected", "too glossy").unwrap();
        assert_eq!(r.verdict, "rejected");
        let item = lib.get_item(r.item_id.as_ref().unwrap()).unwrap();
        assert!(item.anti);
        assert_eq!(item.weight, REJECTED_WEIGHT);
        assert!(item.note.contains("too glossy"));
        assert!(item.tags.contains(&"agent-output".to_string()));

        // Changed mind: accepted, same item, lower weight, no longer anti.
        let a = lib.set_output_verdict(&out.id, "accepted", "").unwrap();
        assert_eq!(a.item_id, r.item_id);
        let item = lib.get_item(a.item_id.as_ref().unwrap()).unwrap();
        assert!(!item.anti);
        assert_eq!(item.weight, ACCEPTED_WEIGHT);
        assert_eq!(lib.list_items(&lang.id).unwrap().len(), 1);

        // Pending again removes the item.
        let p = lib.set_output_verdict(&out.id, "pending", "").unwrap();
        assert!(p.item_id.is_none());
        assert!(lib.list_items(&lang.id).unwrap().is_empty());
        assert!(lib.set_output_verdict(&out.id, "maybe", "").is_err());

        // An image that is already a reference cannot become an output.
        lib.import_bytes(&lang.id, "ref.png", &png(7), None).unwrap();
        let dup = lib.submit_output("quiet", "same.png", &png(7), "", "", "", "", &[]).unwrap();
        assert!(lib.set_output_verdict(&dup.id, "accepted", "").is_err());
        assert_eq!(lib.list_items(&lang.id).unwrap().len(), 1);

        // Removing an output removes its files and its item.
        lib.set_output_verdict(&out.id, "accepted", "").unwrap();
        lib.remove_output(&out.id).unwrap();
        assert!(!lib.abs(&out.file_rel).unwrap().exists());
        assert!(lib.get_output(&out.id).is_err());
        assert_eq!(lib.list_items(&lang.id).unwrap().len(), 1);
        assert_eq!(lib.find_output(&dup.id[..8]).unwrap().id, dup.id);
    }
}
