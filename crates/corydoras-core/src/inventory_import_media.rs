//! Recover product colours and primary photos from a local workshop catalogue archive.
use crate::inventory::normalized_colour;
use crate::inventory_media::{prepare_photo, store_photo, PreparedPhoto};
use crate::Library;
use anyhow::{ensure, Context, Result};
use ring::digest::{digest, SHA256};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const MAX_JSON: u64 = 16 * 1024 * 1024;
const MAX_PHOTO: u64 = 12 * 1024 * 1024;
const MAX_BATCH: usize = 192 * 1024 * 1024;

#[derive(Default, Debug, Serialize)]
pub struct ImportReport {
    pub matched: usize,
    pub archived: usize,
    pub unmatched: usize,
    pub previously_processed: usize,
    pub photos_added: usize,
    pub colours_added: usize,
}

#[derive(Deserialize)]
struct Catalogue {
    items: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    id: String,
    archived: bool,
    photos: Vec<String>,
    attributes: HashMap<String, Attribute>,
}

#[derive(Deserialize)]
struct Attribute {
    value: String,
}

#[derive(Deserialize)]
struct Manifest {
    files: HashMap<String, FileRecord>,
}

#[derive(Deserialize)]
struct FileRecord {
    bytes: u64,
    sha256: String,
}

fn is_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn sha256(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn read_file(root: &Path, relative: &str, limit: u64) -> Result<Vec<u8>> {
    // Callers construct the two fixed JSON names and photos/<validated ID>.jpg.
    let path = root
        .join(relative)
        .canonicalize()
        .with_context(|| format!("Cannot resolve {relative}"))?;
    ensure!(
        path.starts_with(root),
        "Archive path leaves its folder: {relative}"
    );
    let file = File::open(&path).with_context(|| format!("Cannot open {relative}"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "Invalid archive file: {relative}"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Archive file is too large: {relative}"
    );
    Ok(bytes)
}

fn checked(root: &Path, manifest: &Manifest, relative: &str, limit: u64) -> Result<Vec<u8>> {
    let record = manifest
        .files
        .get(relative)
        .with_context(|| format!("No checksum for {relative}"))?;
    let bytes = read_file(root, relative, limit)?;
    ensure!(
        bytes.len() as u64 == record.bytes && sha256(&bytes) == record.sha256,
        "The archive checksum does not match {relative}."
    );
    Ok(bytes)
}

// Track completed imports separately from the user's fields. A repeat must respect a later removal.
const MARKERS: &str = "CREATE TABLE IF NOT EXISTS workshop_media_imports (
    item_id TEXT PRIMARY KEY REFERENCES workshop_inventory(id) ON DELETE CASCADE
)";

struct Candidate {
    id: String,
    colour: String,
    photo: Option<String>,
}

fn candidates(
    library: &Library,
    items: Vec<Entry>,
    report: &mut ImportReport,
) -> Result<Vec<Candidate>> {
    let connection = library.c();
    connection.execute_batch(MARKERS)?;
    let mut state = connection.prepare(
        "SELECT i.photo_key IS NULL, i.colour='', m.item_id IS NOT NULL
         FROM workshop_inventory i LEFT JOIN workshop_media_imports m ON i.id=m.item_id WHERE i.id=?1")?;
    let mut ids = HashSet::new();
    let mut pending = Vec::new();
    for row in items {
        ensure!(
            is_id(&row.id) && ids.insert(row.id.clone()),
            "Invalid or duplicate source item ID."
        );
        if row.archived {
            report.archived += 1;
            continue;
        }
        let status = state
            .query_row([&row.id], |r| {
                Ok((
                    r.get::<_, bool>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })
            .optional()?;
        let Some((need_photo, need_colour, done)) = status else {
            report.unmatched += 1;
            continue;
        };
        report.matched += 1;
        if done {
            report.previously_processed += 1;
            continue;
        }
        ensure!(
            row.photos.iter().all(|id| is_id(id)),
            "Invalid photo ID on item {}.",
            row.id
        );
        let colour = if need_colour {
            row.attributes
                .get("colour")
                .map(|attr| normalized_colour(&attr.value))
                .transpose()?
                .unwrap_or_default()
        } else {
            String::new()
        };
        pending.push(Candidate {
            id: row.id,
            colour,
            photo: if need_photo {
                row.photos.into_iter().next()
            } else {
                None
            },
        });
    }
    Ok(pending)
}

fn photos(
    root: &Path,
    manifest: &Manifest,
    pending: &[Candidate],
) -> Result<BTreeMap<String, PreparedPhoto>> {
    let mut photos = BTreeMap::new();
    let mut total = 0;
    for id in pending.iter().filter_map(|i| i.photo.as_ref()) {
        if photos.contains_key(id) {
            continue;
        }
        let bytes = checked(root, manifest, &format!("photos/{id}.jpg"), MAX_PHOTO)?;
        let photo =
            prepare_photo(&bytes).with_context(|| format!("Cannot read archive photo {id}"))?;
        total += photo.image.len() + photo.thumbnail.len();
        ensure!(
            total <= MAX_BATCH,
            "The archive exceeds the photo import size limit."
        );
        photos.insert(id.clone(), photo);
    }
    Ok(photos)
}

/// Fill missing fields on existing, active source IDs. Import only the first photo for each item.
/// Existing references and all photos stay in the source archive.
pub fn import_workshop_media(library: &Library, archive: &Path) -> Result<ImportReport> {
    let archive = archive
        .canonicalize()
        .context("Cannot open the catalogue archive.")?;
    let manifest = serde_json::from_slice(&read_file(&archive, "manifest.json", MAX_JSON)?)?;
    let catalogue: Catalogue =
        serde_json::from_slice(&checked(&archive, &manifest, "catalogue.json", MAX_JSON)?)?;
    let mut report = ImportReport::default();
    let pending = candidates(library, catalogue.items, &mut report)?;
    let photos = photos(&archive, &manifest, &pending)?;
    let mut connection = library.c();
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut stored = HashSet::new();
    for row in pending {
        // The user could edit or delete an item while the photos decode.
        let marked = tx.execute(
            "INSERT INTO workshop_media_imports(item_id)
            SELECT id FROM workshop_inventory WHERE id=?1 ON CONFLICT DO NOTHING",
            [&row.id],
        )?;
        if marked == 0 {
            continue;
        }
        if !row.colour.is_empty() {
            report.colours_added += tx.execute(
                "UPDATE workshop_inventory SET colour=?2 WHERE id=?1 AND colour=''",
                params![row.id, row.colour],
            )?;
        }
        if let Some(photo) = row.photo.as_ref().and_then(|id| photos.get(id)) {
            let needs_photo: bool = tx.query_row(
                "SELECT photo_key IS NULL FROM workshop_inventory WHERE id=?1",
                [&row.id],
                |r| r.get(0),
            )?;
            if needs_photo {
                if stored.insert(&photo.key) {
                    store_photo(&tx, photo)?;
                }
                report.photos_added += tx.execute(
                    "UPDATE workshop_inventory SET photo_key=?2 WHERE id=?1",
                    params![row.id, photo.key],
                )?;
            }
        }
    }
    tx.commit()?;
    Ok(report)
}

#[cfg(test)]
mod tests;
