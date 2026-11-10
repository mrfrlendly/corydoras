//! Where the library lives: checks for a new folder, and a safe move.
//!
//! A move copies the files, writes a consistent copy of the database with
//! VACUUM INTO, opens the copy and checks it, and only then reports success.
//! The old folder stays untouched until the caller removes it.

use crate::Library;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const DB_FILE: &str = "library.sqlite";

/// True when the folder holds a Corydoras library.
pub fn is_library(path: &Path) -> bool {
    path.join(DB_FILE).is_file()
}

/// Information about a folder that you choose.
#[derive(Debug, Serialize, Clone)]
pub struct FolderInfo {
    pub path: String,
    pub exists: bool,
    pub is_library: bool,
    /// True when the folder has no files (or does not exist).
    pub empty: bool,
    /// A warning about cloud-synced folders, if one applies.
    pub warning: Option<String>,
}

/// Files of the system that do not make a folder "not empty".
fn ignorable(name: &str) -> bool {
    matches!(name, ".DS_Store" | ".localized" | "Icon\r")
}

pub fn folder_info(path: &Path) -> FolderInfo {
    let exists = path.is_dir();
    let empty = !exists
        || std::fs::read_dir(path)
            .map(|rd| rd.flatten().all(|e| ignorable(&e.file_name().to_string_lossy())))
            .unwrap_or(false);
    FolderInfo {
        path: path.to_string_lossy().into_owned(),
        exists,
        is_library: is_library(path),
        empty,
        warning: cloud_warning(path),
    }
}

/// SQLite databases and sync services do not mix well: a sync can copy the
/// database in the middle of a write, or remove local copies to save space.
pub fn cloud_warning(path: &Path) -> Option<String> {
    let p = path.to_string_lossy();
    let service = if p.contains("/Library/Mobile Documents/") {
        Some("iCloud Drive")
    } else if p.contains("/Library/CloudStorage/") || p.contains("/Dropbox") {
        Some(if p.contains("Dropbox") {
            "Dropbox"
        } else if p.contains("GoogleDrive") || p.contains("Google Drive") {
            "Google Drive"
        } else if p.contains("OneDrive") {
            "OneDrive"
        } else {
            "a cloud service"
        })
    } else {
        None
    };
    service.map(|s| {
        format!(
            "This folder syncs with {s}. A sync can damage the library database while the app writes to it, \
             and the service can remove the local copies of images. A local folder (or an external drive) is safer; \
             keep backups with Time Machine."
        )
    })
}

/// Checks that `dst` can receive the library that is at `src`.
pub fn check_move_target(src: &Path, dst: &Path) -> Result<()> {
    // Resolve symlinks through the nearest folder that exists: a new target folder
    // does not exist yet, and comparing it unresolved against a resolved library
    // path (for example /var against /private/var) would miss a move into itself.
    let norm = |p: &Path| {
        let mut rest = Vec::new();
        let mut base = p.to_path_buf();
        loop {
            if let Ok(real) = base.canonicalize() {
                return rest.iter().rev().fold(real, |acc: PathBuf, part| acc.join(part));
            }
            match (base.file_name().map(|n| n.to_os_string()), base.parent()) {
                (Some(name), Some(parent)) => {
                    rest.push(name);
                    base = parent.to_path_buf();
                }
                _ => return p.to_path_buf(),
            }
        }
    };
    let (s, d) = (norm(src), norm(dst));
    if s == d {
        bail!("The library is in that folder already.");
    }
    if d.starts_with(&s) {
        bail!("Choose a folder outside the current library folder.");
    }
    if s.starts_with(&d) {
        bail!("Choose a folder that is not a parent of the current library folder.");
    }
    let info = folder_info(dst);
    if info.is_library {
        bail!("That folder holds another library. Use \"Open another library\" to switch to it.");
    }
    if !info.empty {
        bail!("Choose an empty folder, or a new folder. The folder that you chose has files in it.");
    }
    let parent = dst.parent().ok_or_else(|| anyhow!("Choose a folder, not the disk root."))?;
    if !parent.is_dir() {
        bail!("The folder {} does not exist.", parent.display());
    }
    Ok(())
}

/// Files and bytes in a folder (the database files excluded).
pub fn measure(src: &Path) -> (u64, u64) {
    let mut files = 0;
    let mut bytes = 0;
    walk(src, &mut |p, _| {
        files += 1;
        bytes += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    });
    (files, bytes)
}

/// Calls `f` for each regular file below `root`, with its path relative to root.
/// It skips the SQLite files (they are copied with VACUUM INTO) and symbolic links.
pub(crate) fn walk(root: &Path, f: &mut dyn FnMut(&Path, &Path)) {
    fn inner(root: &Path, dir: &Path, f: &mut dyn FnMut(&Path, &Path)) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                inner(root, &p, f);
            } else if ft.is_file() {
                let rel = p.strip_prefix(root).unwrap_or(&p).to_path_buf();
                let name = rel.to_string_lossy();
                if name == DB_FILE || name.starts_with("library.sqlite-") {
                    continue;
                }
                f(&p, &rel);
            }
        }
    }
    inner(root, root, f)
}

#[derive(Debug, Serialize, Clone)]
pub struct MoveReport {
    pub new_path: String,
    pub files: u64,
    pub bytes: u64,
}

/// Copies the library of `lib` to `dst` and checks the copy. `progress(done, total)`
/// reports bytes. On an error, the partial copy is removed.
pub fn copy_library(lib: &Library, dst: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<MoveReport> {
    let src = lib.root().to_path_buf();
    check_move_target(&src, dst)?;
    let created = !dst.exists();
    let result = (|| -> Result<MoveReport> {
        std::fs::create_dir_all(dst).with_context(|| format!("Cannot make {}", dst.display()))?;
        let (files, total) = measure(&src);
        let mut done = 0u64;
        let mut copied = 0u64;
        let mut err: Option<anyhow::Error> = None;
        walk(&src, &mut |from, rel| {
            if err.is_some() {
                return;
            }
            let to = dst.join(rel);
            let r = (|| -> Result<()> {
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let n = std::fs::copy(from, &to).with_context(|| format!("Cannot copy {}", rel.display()))?;
                let want = std::fs::metadata(from)?.len();
                if n != want {
                    bail!("The copy of {} is incomplete.", rel.display());
                }
                done += n;
                copied += 1;
                Ok(())
            })();
            match r {
                Ok(()) => progress(done, total),
                Err(e) => err = Some(e),
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
        lib.copy_database_to(&dst.join(DB_FILE)).context("Cannot copy the database")?;
        let check = Library::open(dst).context("Cannot open the copied library")?;
        if !check.integrity_ok()? {
            bail!("The copied database failed its check.");
        }
        let a = lib.list_languages()?.len();
        let b = check.list_languages()?.len();
        if a != b {
            bail!("The copy has {b} languages; the original has {a}.");
        }
        if copied != files {
            bail!("Copied {copied} of {files} files.");
        }
        Ok(MoveReport { new_path: dst.to_string_lossy().into_owned(), files, bytes: total })
    })();
    if result.is_err() {
        // Remove what this move made. A folder that existed before stays, empty.
        if created {
            let _ = std::fs::remove_dir_all(dst);
        } else if let Ok(rd) = std::fs::read_dir(dst) {
            for e in rd.flatten() {
                let p = e.path();
                let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            }
        }
    }
    result
}

/// Moves the old library folder to the Trash.
pub fn trash_folder(path: &Path) -> Result<()> {
    if !is_library(path) {
        bail!("{} is not a library folder; it stays where it is.", path.display());
    }
    #[allow(unused_mut)]
    let mut ctx = trash::TrashContext::default();
    // The file manager call, not Finder: Finder would ask for Automation permission.
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx.delete(path).map_err(|e| anyhow!("Cannot move {} to the Trash: {e}", path.display()))
}

/// A folder next to `path` with a free name: "Corydoras 2", "Corydoras 3", ...
pub fn free_sibling(path: &Path) -> PathBuf {
    let mut n = 2;
    let base = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Corydoras".into());
    loop {
        let p = path.with_file_name(format!("{base} {n}"));
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_library_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("lib");
        let lib = Library::open(&src).unwrap();
        let lang = lib.create_language("Quiet", "").unwrap();
        // Any file inside the library is copied.
        std::fs::create_dir_all(src.join("languages").join(&lang.id).join("items")).unwrap();
        std::fs::write(src.join("languages").join(&lang.id).join("items/a.jpg"), vec![7u8; 5000]).unwrap();

        assert!(check_move_target(&src, &src).is_err());
        assert!(check_move_target(&src, &src.join("inner")).is_err());
        let full = tmp.path().join("full");
        std::fs::create_dir_all(&full).unwrap();
        std::fs::write(full.join("x.txt"), "x").unwrap();
        assert!(check_move_target(&src, &full).is_err());

        let dst = tmp.path().join("moved");
        let mut last = (0, 0);
        let r = copy_library(&lib, &dst, &mut |d, t| last = (d, t)).unwrap();
        assert!(r.files >= 1 && last.0 == last.1 && last.1 >= 5000);
        let copy = Library::open(&dst).unwrap();
        assert_eq!(copy.list_languages().unwrap()[0].name, "Quiet");
        assert_eq!(std::fs::read(dst.join("languages").join(&lang.id).join("items/a.jpg")).unwrap().len(), 5000);
        // The target is now a library: a second move there is refused.
        assert!(check_move_target(&src, &dst).is_err());
        assert!(cloud_warning(Path::new("/Users/t/Library/Mobile Documents/com~apple~CloudDocs/Cory")).is_some());
        assert!(cloud_warning(Path::new("/Users/t/Corydoras")).is_none());
        assert_eq!(free_sibling(&dst).file_name().unwrap(), "moved 2");
    }
}
