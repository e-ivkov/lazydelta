//! Project open / refresh flow (design D2/D3): derive a changeset for a
//! directory (git HEAD base, snapshot fallback), carry review state across
//! refreshes, and manage the thread document lifecycle.

use std::io;
use std::path::{Path, PathBuf};

use lazydelta_core::{ChangeSet, Provenance, git, snapshot};

use crate::store::{ThreadDocument, ThreadStore};

#[derive(Debug, Clone, PartialEq)]
pub enum BaseMode {
    Git,
    Snapshot,
}

/// How a project's changeset is derived and persisted.
pub struct Project {
    pub root: PathBuf,
    pub thread_id: String,
    base_mode: BaseMode,
}

impl Project {
    /// Open (or create) the review thread for `root` and return the current
    /// changeset plus the persisted document, if one existed.
    pub fn open(
        root: &Path,
        thread_id: &str,
        force_snapshot: bool,
    ) -> io::Result<(Project, ChangeSet, Option<ThreadDocument>)> {
        let store = ThreadStore::new(root);
        let existing = store.load(thread_id).ok();

        let (base_mode, changeset) = if git::is_git_repo(root) && !force_snapshot {
            (BaseMode::Git, git::derive_from_git(root, agent_origin())?)
        } else {
            // Snapshot mode: capture the current state as base on first open.
            let snap_dir = Self::snapshot_dir(root, thread_id);
            if !snap_dir.is_dir() {
                let current = snapshot::read_current(root)?;
                std::fs::create_dir_all(&snap_dir)?;
                snapshot::write_snapshot(&snap_dir, &current)?;
            }
            let base = snapshot::read_snapshot(&snap_dir)?;
            let current = snapshot::read_current(root)?;
            (BaseMode::Snapshot, ChangeSet::from_contents(&base, &current, Provenance::Human))
        };

        let mut cs = changeset;
        if let Some(doc) = &existing {
            let _warnings = cs.carry_state_from(&doc.changeset);
        }
        let project = Project {
            root: root.to_path_buf(),
            thread_id: thread_id.to_string(),
            base_mode,
        };
        Ok((project, cs, existing))
    }

    fn snapshot_dir(root: &Path, thread_id: &str) -> PathBuf {
        root.join(".lazydelta").join("threads").join(thread_id).join("base")
    }

    pub fn base_mode(&self) -> &BaseMode {
        &self.base_mode
    }

    /// Re-derive the changeset from disk, carrying review state over.
    /// Returns (changeset, warnings).
    pub fn refresh(&self, old: &ChangeSet) -> io::Result<(ChangeSet, Vec<String>)> {
        let fresh = match self.base_mode {
            BaseMode::Git => git::derive_from_git(&self.root, agent_origin())?,
            BaseMode::Snapshot => {
                let base = snapshot::read_snapshot(&Self::snapshot_dir(&self.root, &self.thread_id))?;
                let current = snapshot::read_current(&self.root)?;
                ChangeSet::from_contents(&base, &current, Provenance::Human)
            }
        };
        let mut cs = fresh;
        let warnings = cs.carry_state_from(old);
        Ok((cs, warnings))
    }

    /// Compare a file on disk with the preview materialization; `true` when
    /// they match (used by the editor-jump content guarantee).
    pub fn disk_matches_preview(&self, _cs: &ChangeSet, path: &Path, preview: &str) -> bool {
        match std::fs::read(self.root.join(path)) {
            Ok(bytes) => String::from_utf8_lossy(&bytes) == preview,
            Err(_) => false,
        }
    }
}

fn agent_origin() -> Provenance {
    Provenance::Agent { agent: "thread-agent".into(), turn: 1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazydelta_core::{HunkStatus, LineKind};

    fn git_repo_fixture(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ld-proj-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(&args)
                .output()
                .unwrap();
        }
        std::fs::write(root.join("src/a.rs"), "one\ntwo\n").unwrap();
        std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", "-A"])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["commit", "-qm", "base"])
            .output()
            .unwrap();
        root
    }

    #[test]
    fn open_derives_git_and_carries_state_on_refresh() {
        let root = git_repo_fixture("git");
        std::fs::write(root.join("src/a.rs"), "one\nTWO\n").unwrap();

        let (project, cs, existing) = Project::open(&root, "main", false).unwrap();
        assert_eq!(project.base_mode(), &BaseMode::Git);
        assert!(existing.is_none());
        assert!(cs.files.iter().any(|f| f.path == Path::new("src/a.rs")));

        // Accept the hunk, persist, then edit externally and refresh.
        let mut cs2 = cs.clone();
        let hunk_id = cs2.files[0].hunks[0].id;
        let _ = cs2.accept_hunk(hunk_id);
        let mut doc = ThreadDocument::new("main", cs2.clone());
        ThreadStore::new(&root).save(&doc).unwrap();

        std::fs::write(root.join("src/a.rs"), "one\nTWO!\nthree\n").unwrap();
        let (fresh, warnings) = project.refresh(&cs2).unwrap();
        assert!(warnings.iter().all(|w| !w.is_empty()));
        // The refreshed hunk (same key region) keeps the accepted status...
        let any_accepted = fresh.files.iter().any(|f| {
            f.hunks
                .iter()
                .any(|h| h.status == HunkStatus::Accepted)
        });
        assert!(any_accepted, "warnings: {warnings:?}");
        // ...and the external edit shows up as an added line.
        assert!(fresh.files.iter().any(|f| {
            f.hunks.iter().any(|h| h.lines.iter().any(|l| {
                l.kind == LineKind::Added && l.text.contains("three")
            }))
        }));

        doc.changeset = fresh.clone();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn snapshot_mode_captures_base_on_first_open() {
        let root = std::env::temp_dir().join(format!("ld-proj-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "v1\n").unwrap();

        // First open: no changes yet, snapshot captured.
        let (project, cs, _) = Project::open(&root, "main", false).unwrap();
        assert_eq!(project.base_mode(), &BaseMode::Snapshot);
        assert!(cs.files.is_empty());
        assert!(root.join(".lazydelta/threads/main/base/a.txt").is_file());

        // Edit externally, refresh → human-provenance hunk.
        std::fs::write(root.join("a.txt"), "v1\nv2\n").unwrap();
        let (fresh, _) = project.refresh(&cs).unwrap();
        assert_eq!(fresh.files.len(), 1);
        assert_eq!(fresh.files[0].hunks.first().map(|h| h.provenance.clone()), Some(Provenance::Human));

        let _ = std::fs::remove_dir_all(&root);
    }
}

