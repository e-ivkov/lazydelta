//! Snapshot fallback for non-git directories (design D2): capture the base
//! state once at thread creation, re-derive against it afterwards.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// Write `files` as the base snapshot under `dst` (mirroring relative paths).
pub fn write_snapshot(dst: &Path, files: &BTreeMap<PathBuf, String>) -> io::Result<()> {
    for (rel, text) in files {
        let target = dst.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, text)?;
    }
    Ok(())
}

/// Read a base snapshot back from `src` (all files below it, relative to `src`).
pub fn read_snapshot(src: &Path) -> io::Result<BTreeMap<PathBuf, String>> {
    let mut files = BTreeMap::new();
    fn walk(dir: &Path, prefix: &Path, files: &mut BTreeMap<PathBuf, String>) -> io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name();
                let sub = prefix.join(&name);
                walk(&path, &sub, files)?;
            } else {
                let bytes = std::fs::read(&path)?;
                let rel = prefix.join(entry.file_name());
                files.insert(rel, String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        Ok(())
    }
    walk(src, Path::new(""), &mut files)?;
    Ok(files)
}

/// Read the current contents of all regular files under `root` (relative
/// paths), skipping `.lazydelta/` and dot-directories.
pub fn read_current(root: &Path) -> io::Result<BTreeMap<PathBuf, String>> {
    let mut files = BTreeMap::new();
    fn walk(dir: &Path, prefix: &Path, files: &mut BTreeMap<PathBuf, String>) -> io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let rel = prefix.join(&name);
            if path.is_dir() {
                walk(&path, &rel, files)?;
            } else {
                let bytes = std::fs::read(&path)?;
                files.insert(rel, String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        Ok(())
    }
    walk(root, Path::new(""), &mut files)?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChangeSet;

    #[test]
    fn snapshot_roundtrip_and_derive() {
        let root = std::env::temp_dir().join(format!("ld-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "one\ntwo\n").unwrap();

        let base = read_current(&root).unwrap();
        let snap_dir = root.join(".lazydelta").join("threads").join("main").join("base");
        std::fs::create_dir_all(&snap_dir).unwrap();
        write_snapshot(&snap_dir, &base).unwrap();
        assert_eq!(read_snapshot(&snap_dir).unwrap().get(&PathBuf::from("src/a.rs")).unwrap(), "one\ntwo\n");

        // Edit, then derive against the snapshot.
        std::fs::write(root.join("src/a.rs"), "one\nTWO\n").unwrap();
        let snap = read_snapshot(&snap_dir).unwrap();
        let current = read_current(&root).unwrap();
        let cs = ChangeSet::from_contents(&snap, &current, crate::Provenance::Human);
        assert_eq!(cs.files.len(), 1);
        assert!(cs.files[0].hunks[0]
            .lines
            .iter()
            .any(|l| l.kind == crate::LineKind::Added && l.text == "TWO"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
