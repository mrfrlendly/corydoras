//! Library snapshots: a safety net beside the warning about synced folders.
//!
//! - A snapshot is one zip file with a consistent copy of the database and
//!   every file of the library. It can go to any folder, for example an
//!   external drive, and it opens again as a library.
//! - Once a day the app also keeps a copy of the database alone in
//!   `.snapshots/` inside the library (the newest seven). The images do not
//!   change after import, so the database is the part that a broken sync or a
//!   crash can damage.

use crate::storage;
use crate::Library;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Daily database copies to keep.
pub const KEEP_DB_COPIES: usize = 7;
const DB_DIR: &str = ".snapshots";
/// File types that are compressed already.
const PACKED: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "pdf", "zip", "3mf", "mp4", "mov", "m4v", "docx", "odt"];

#[derive(Debug, Serialize, Clone)]
pub struct SnapshotReport {
    pub path: String,
    pub files: u64,
    /// Size of the zip file.
    pub bytes: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct DbCopy {
    pub name: String,
    pub path: String,
    pub bytes: u64,
}

fn skipped(rel: &Path) -> bool {
    let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
    first == ".tmp" || first == DB_DIR || rel.file_name().map(|n| n == ".DS_Store").unwrap_or(false)
}

/// Writes a snapshot of the library into `dest_dir`. `progress(done, total)` reports files.
pub fn create(lib: &Library, dest_dir: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<SnapshotReport> {
    if !dest_dir.is_dir() {
        bail!("The folder {} does not exist.", dest_dir.display());
    }
    let (root, dest) = (lib.root().canonicalize().unwrap_or_else(|_| lib.root().to_path_buf()), dest_dir.canonicalize().unwrap_or_else(|_| dest_dir.to_path_buf()));
    if dest.starts_with(&root) {
        bail!("Choose a folder outside the library. A snapshot inside the library does not protect it.");
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M%S").to_string();
    let zip_path = dest.join(format!("Corydoras snapshot {stamp}.zip"));
    if zip_path.exists() {
        bail!("{} exists already.", zip_path.display());
    }
    let db_copy = dest.join(format!(".corydoras-snapshot-{}.sqlite", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<SnapshotReport> {
        lib.copy_database_to(&db_copy).context("Cannot copy the database")?;
        let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();
        storage::walk(&root, &mut |abs, rel| {
            if !skipped(rel) {
                files.push((abs.to_path_buf(), rel.to_path_buf()));
            }
        });
        files.sort();
        let total = files.len() as u64 + 2;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).with_context(|| format!("Cannot make {}", zip_path.display()))?);
        let options = |name: &str, size: u64| {
            let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            zip::write::SimpleFileOptions::default()
                .compression_method(if PACKED.contains(&ext.as_str()) { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated })
                .large_file(size >= u32::MAX as u64)
        };
        let manifest = serde_json::json!({
            "app": "Corydoras",
            "version": env!("CARGO_PKG_VERSION"),
            "created": crate::util::now_iso(),
            "languages": lib.list_languages()?.len(),
            "files": files.len(),
        });
        zip.start_file("snapshot.json", options("snapshot.json", 0))?;
        zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        progress(1, total);
        let db_size = std::fs::metadata(&db_copy)?.len();
        zip.start_file(storage::DB_FILE, options(storage::DB_FILE, db_size))?;
        std::io::copy(&mut std::fs::File::open(&db_copy)?, &mut zip)?;
        progress(2, total);
        for (n, (abs, rel)) in files.iter().enumerate() {
            let name = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            let size = std::fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
            zip.start_file(name.as_str(), options(&name, size))?;
            std::io::copy(&mut std::fs::File::open(abs).with_context(|| format!("Cannot read {}", rel.display()))?, &mut zip)?;
            progress(n as u64 + 3, total);
        }
        zip.finish()?;
        Ok(SnapshotReport { path: zip_path.to_string_lossy().into_owned(), files: files.len() as u64, bytes: std::fs::metadata(&zip_path)?.len() })
    })();
    let _ = std::fs::remove_file(&db_copy);
    if result.is_err() {
        let _ = std::fs::remove_file(&zip_path);
    }
    result
}

/// Opens a snapshot as a new library in `dest`, a folder that is empty or new.
/// Returns the number of files.
pub fn restore(zip_path: &Path, dest: &Path) -> Result<u64> {
    let info = storage::folder_info(dest);
    if !info.empty {
        bail!("Choose an empty folder, or a new folder. The folder that you chose has files in it.");
    }
    let file = std::fs::File::open(zip_path).with_context(|| format!("Cannot open {}", zip_path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| anyhow!("{} is not a zip file.", zip_path.display()))?;
    if zip.by_name(storage::DB_FILE).is_err() {
        bail!("This zip file is not a Corydoras snapshot: it has no {}.", storage::DB_FILE);
    }
    let created = !dest.exists();
    let result = (|| -> Result<u64> {
        std::fs::create_dir_all(dest)?;
        let mut files = 0;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            // A name that would leave the folder is refused.
            let rel = entry.enclosed_name().ok_or_else(|| anyhow!("The snapshot has a file with an unsafe name."))?;
            let out = dest.join(&rel);
            if entry.is_dir() {
                std::fs::create_dir_all(&out)?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::File::create(&out)?;
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = entry.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                f.write_all(&buf[..n])?;
            }
            files += 1;
        }
        let _ = std::fs::remove_file(dest.join("snapshot.json"));
        let lib = Library::open(dest).context("Cannot open the restored library")?;
        if !lib.integrity_ok()? {
            bail!("The database of the snapshot failed its check.");
        }
        Ok(files)
    })();
    if result.is_err() {
        if created {
            let _ = std::fs::remove_dir_all(dest);
        } else if let Ok(rd) = std::fs::read_dir(dest) {
            for e in rd.flatten() {
                let p = e.path();
                let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            }
        }
    }
    result
}

/// The daily database copies of a library, newest first.
pub fn db_copies(root: &Path) -> Vec<DbCopy> {
    let mut out: Vec<DbCopy> = std::fs::read_dir(root.join(DB_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (name.starts_with("library-") && name.ends_with(".sqlite")).then(|| DbCopy {
                path: e.path().to_string_lossy().into_owned(),
                bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// Keeps one copy of the database for each day and removes the oldest ones.
/// Returns the new copy, or None when today has one already.
pub fn daily_db_copy(lib: &Library) -> Result<Option<PathBuf>> {
    let dir = lib.root().join(DB_DIR);
    let path = dir.join(format!("library-{}.sqlite", chrono::Local::now().format("%Y-%m-%d")));
    if path.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(&dir)?;
    lib.copy_database_to(&path)?;
    for old in db_copies(lib.root()).into_iter().skip(KEEP_DB_COPIES) {
        let _ = std::fs::remove_file(old.path);
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("lib");
        let lib = Library::open(&root).unwrap();
        let lang = lib.create_language("Quiet", "").unwrap();
        let img = image::RgbImage::from_fn(32, 24, |x, y| image::Rgb([x as u8, y as u8, 9]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let item = lib.import_bytes(&lang.id, "a.png", &buf.into_inner(), None).unwrap();
        std::fs::create_dir_all(root.join(".tmp")).unwrap();
        std::fs::write(root.join(".tmp/scratch.bin"), b"x").unwrap();

        // A snapshot inside the library is refused.
        assert!(create(&lib, &root, &mut |_, _| {}).is_err());
        assert!(create(&lib, &tmp.path().join("missing"), &mut |_, _| {}).is_err());
        let out = tmp.path().join("backups");
        std::fs::create_dir_all(&out).unwrap();
        let mut last = (0, 0);
        let rep = create(&lib, &out, &mut |d, t| last = (d, t)).unwrap();
        assert_eq!(rep.files, 2, "the image and its thumbnail; the scratch file is left out");
        assert_eq!(last.0, last.1);
        assert!(rep.path.ends_with(".zip") && rep.bytes > 0);
        // Only the zip stays in the folder.
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1);

        // The snapshot opens as a library with the same content.
        let dest = tmp.path().join("restored");
        assert_eq!(restore(Path::new(&rep.path), &dest).unwrap(), 4);
        let copy = Library::open(&dest).unwrap();
        assert_eq!(copy.list_languages().unwrap()[0].name, "Quiet");
        assert!(copy.abs(&copy.get_item(&item.id).unwrap().file_rel).unwrap().exists());
        assert!(!dest.join(".tmp").exists() && !dest.join("snapshot.json").exists());
        // Not into a folder with files, and not from a file that is no snapshot.
        assert!(restore(Path::new(&rep.path), &dest).is_err());
        let other = out.join("other.zip");
        std::fs::write(&other, crate::mesh::tests::zip_of(&[("readme.txt", "hello")])).unwrap();
        assert!(restore(&other, &tmp.path().join("nope")).unwrap_err().to_string().contains("not a Corydoras snapshot"));
        assert!(!tmp.path().join("nope").exists());

        // One database copy for each day.
        let first = daily_db_copy(&lib).unwrap().unwrap();
        assert!(first.exists());
        assert!(daily_db_copy(&lib).unwrap().is_none());
        for d in 1..=9 {
            std::fs::write(root.join(DB_DIR).join(format!("library-2020-01-{d:02}.sqlite")), b"old").unwrap();
        }
        std::fs::remove_file(&first).unwrap();
        daily_db_copy(&lib).unwrap().unwrap();
        let copies = db_copies(&root);
        assert_eq!(copies.len(), KEEP_DB_COPIES);
        assert!(copies[0].name > copies[1].name && copies.iter().all(|c| c.name != "library-2020-01-01.sqlite"));
    }
}
