//! Minimal read-only git integration via the `git` CLI (design D2).

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> io::Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether `root` is inside a git work tree with readable metadata.
pub fn is_git_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

/// The file content at `HEAD` for `path`, or `None` if it is not tracked.
pub fn head_file(root: &Path, path: &Path) -> io::Result<Option<String>> {
    let spec = path.to_string_lossy().into_owned();
    match git(root, &["show", &format!("HEAD:{spec}")]) {
        Ok(text) => Ok(Some(text)),
        Err(_) => Ok(None), // untracked or new file
    }
}

/// Working-tree paths that differ from `HEAD` (modified, added, untracked,
/// renamed — the rename destination is kept). Uninteresting paths
/// (`.lazydelta/`, submodules) are filtered out.
pub fn changed_paths(root: &Path) -> io::Result<Vec<PathBuf>> {
    let out = git(root, &["status", "--porcelain", "-z"])?;
    let mut paths = Vec::new();
    for entry in out.split('\0') {
        if entry.is_empty() {
            continue;
        }
        let Some(rest) = entry.strip_prefix("R  ") else {
            // Porcelain v1: two status chars, one space, then the path.
            let path = entry.get(3..).unwrap_or("").to_string();
            if !path.is_empty() {
                paths.push(PathBuf::from(path));
            }
            continue;
        };
        // Rename entries are "R  new -> old" in -z? In -z the rename source
        // is a separate NUL-terminated field; the first field is the dest.
        let dest = rest.rsplit(" -> ").next().unwrap_or(rest);
        paths.push(PathBuf::from(dest.to_string()));
    }
    paths.retain(|p| {
        let s = p.to_string_lossy();
        !s.starts_with(".lazydelta/")
    });
    Ok(paths)
}

/// Read the current contents of `paths` under `root` (UTF-8 lossy).
/// Files missing on disk (deleted since the status call) are skipped and
/// reported in the second return value — v1 does not model deletions.
pub fn read_worktree(
    root: &Path,
    paths: &[PathBuf],
) -> io::Result<(BTreeMap<PathBuf, String>, Vec<PathBuf>)> {
    let mut contents = BTreeMap::new();
    let mut deleted = Vec::new();
    for p in paths {
        let full = root.join(p);
        match std::fs::read(&full) {
            Ok(bytes) => {
                contents.insert(p.clone(), String::from_utf8_lossy(&bytes).into_owned());
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => deleted.push(p.clone()),
            Err(e) => return Err(e),
        }
    }
    Ok((contents, deleted))
}

/// Derive a changeset for a git worktree: base = `HEAD`, proposed = disk.
pub fn derive_from_git(root: &Path, origin: crate::Provenance) -> io::Result<crate::ChangeSet> {
    let paths = changed_paths(root)?;
    let (current, _deleted) = read_worktree(root, &paths)?;
    let mut base = BTreeMap::new();
    for p in current.keys() {
        if let Some(text) = head_file(root, p)? {
            base.insert(p.clone(), text);
        }
    }
    Ok(crate::ChangeSet::from_contents(&base, &current, origin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LineKind, MaterializePolicy};

    fn init_repo(dir: &Path) {
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            git(dir, &args).unwrap();
        }
    }

    fn commit_all(dir: &Path, msg: &str) {
        git(dir, &["add", "-A"]).unwrap();
        git(dir, &["commit", "-qm", msg]).unwrap();
    }

    #[test]
    fn derives_modified_and_new_files_from_head() {
        let root = std::env::temp_dir().join(format!("ld-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        init_repo(&root);
        std::fs::write(root.join("src/a.rs"), "one\ntwo\nthree\n").unwrap();
        commit_all(&root, "base");

        std::fs::write(root.join("src/a.rs"), "one\nTWO\nthree\nfour\n").unwrap();
        std::fs::write(root.join("src/new.rs"), "hello\n").unwrap();

        let cs = derive_from_git(&root, crate::Provenance::Agent { agent: "t".into(), turn: 1 }).unwrap();
        assert_eq!(cs.files.len(), 2);

        let a = cs.files.iter().find(|f| f.path == Path::new("src/a.rs")).unwrap();
        assert!(a.hunks.iter().any(|h| h.lines.iter().any(|l| l.kind == LineKind::Added && l.text == "TWO")));
        // base side really came from HEAD:
        let out = cs.materialize(MaterializePolicy::Proposed);
        assert_eq!(out.get(Path::new("src/new.rs")).unwrap(), "hello\n");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn changed_paths_filters_lazydelta() {
        let root = std::env::temp_dir().join(format!("ld-git-f-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".lazydelta/threads/main")).unwrap();
        init_repo(&root);
        std::fs::write(root.join("a.txt"), "x\n").unwrap();
        commit_all(&root, "base");
        std::fs::write(root.join(".lazydelta/threads/main/thread.json"), "{}").unwrap();
        std::fs::write(root.join("b.txt"), "y\n").unwrap();

        let paths = changed_paths(&root).unwrap();
        assert!(paths.contains(&PathBuf::from("b.txt")));
        assert!(!paths.iter().any(|p| p.starts_with(".lazydelta")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn is_git_repo_false_for_plain_dir() {
        let root = std::env::temp_dir().join(format!("ld-plain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(!is_git_repo(&root));
        let _ = std::fs::remove_dir_all(&root);
    }
}
