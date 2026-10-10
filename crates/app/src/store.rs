//! Thread persistence (design D3): one JSON document per thread under
//! `.lazydelta/threads/<id>/thread.json`, written atomically (tmp + rename),
//! with loud failures on unknown schema versions.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use lazydelta_core::ChangeSet;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentMessage {
    /// "you" | "agent" | "system"
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThreadDocument {
    pub schema_version: u32,
    pub id: String,
    pub changeset: ChangeSet,
    #[serde(default)]
    pub messages: Vec<AgentMessage>,
    /// Prompts accepted by the UI but not yet delivered (durability).
    #[serde(default)]
    pub queue: Vec<String>,
    #[serde(default)]
    pub collapsed: bool,
}

impl ThreadDocument {
    pub fn new(id: impl Into<String>, changeset: ChangeSet) -> Self {
        ThreadDocument {
            schema_version: SCHEMA_VERSION,
            id: id.into(),
            changeset,
            messages: Vec::new(),
            queue: Vec::new(),
            collapsed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadLoadError {
    pub path: PathBuf,
    pub reason: String,
}

impl std::fmt::Display for ThreadLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.reason)
    }
}

pub struct ThreadStore {
    threads_dir: PathBuf,
}

impl ThreadStore {
    pub fn new(project_root: &Path) -> Self {
        ThreadStore {
            threads_dir: project_root.join(".lazydelta").join("threads"),
        }
    }

    fn thread_dir(&self, id: &str) -> PathBuf {
        self.threads_dir.join(id)
    }

    fn thread_path(&self, id: &str) -> PathBuf {
        self.thread_dir(id).join("thread.json")
    }

    /// Load every thread; files that fail to load are reported in the second
    /// return value (with file + reason) and never abort the others.
    pub fn load_all(&self) -> (Vec<ThreadDocument>, Vec<ThreadLoadError>) {
        let mut docs = Vec::new();
        let mut errors = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.threads_dir) else {
            return (docs, errors);
        };
        for entry in entries.flatten() {
            let path = entry.path().join("thread.json");
            if !path.is_file() {
                continue;
            }
            match Self::load_path(&path) {
                Ok(doc) => docs.push(doc),
                Err(reason) => errors.push(ThreadLoadError { path, reason }),
            }
        }
        docs.sort_by(|a, b| a.id.cmp(&b.id));
        (docs, errors)
    }

    pub fn load(&self, id: &str) -> Result<ThreadDocument, ThreadLoadError> {
        let path = self.thread_path(id);
        Self::load_path(&path).map_err(|reason| ThreadLoadError { path, reason })
    }

    fn load_path(path: &Path) -> Result<ThreadDocument, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read: {e}"))?;
        // Version check before typed parse so future docs fail loudly
        // with both versions named (spec: thread-persistence).
        let raw: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON: {e}"))?;
        let declared = raw
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if declared > SCHEMA_VERSION as u64 {
            return Err(format!(
                "thread schema version {declared} is newer than supported version {SCHEMA_VERSION}"
            ));
        }
        if declared == 0 {
            return Err("thread is missing schema_version".to_string());
        }
        serde_json::from_value(raw).map_err(|e| format!("does not match schema v{SCHEMA_VERSION}: {e}"))
    }

    /// Save atomically: write `thread.json.tmp`, then rename over the target.
    pub fn save(&self, doc: &ThreadDocument) -> io::Result<()> {
        let dir = self.thread_dir(&doc.id);
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join("thread.json.tmp");
        let json = serde_json::to_vec_pretty(doc)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, self.thread_path(&doc.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazydelta_core::Provenance;

    fn tmp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ld-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn sample(id: &str) -> ThreadDocument {
        let cs = ChangeSet::from_diff(
            "src/a.rs",
            "one\ntwo\n",
            "one\nTWO\n",
            Provenance::Agent { agent: "t".into(), turn: 1 },
        );
        let mut doc = ThreadDocument::new(id, cs);
        doc.messages.push(AgentMessage {
            role: "agent".into(),
            text: "hello **world**".into(),
        });
        doc.collapsed = true;
        doc
    }

    #[test]
    fn roundtrip_preserves_everything() {
        let root = tmp_root("rt");
        let store = ThreadStore::new(&root);
        let doc = sample("main");
        store.save(&doc).unwrap();
        let back = store.load("main").unwrap();
        assert_eq!(back, doc);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn save_is_atomic_and_ignores_stale_tmp() {
        let root = tmp_root("atomic");
        let store = ThreadStore::new(&root);
        store.save(&sample("main")).unwrap();
        // Simulate a crashed save: leftover tmp, truncated.
        std::fs::write(
            root.join(".lazydelta").join("threads").join("main").join("thread.json.tmp"),
            "{\"schema_version\":1,\"id\":\"ma",
        )
        .unwrap();
        // Loading ignores tmp files entirely (they are not thread.json).
        assert!(store.load("main").is_ok());
        // A subsequent save overwrites the tmp and renames cleanly.
        let mut doc = sample("main");
        doc.queue.push("pending".into());
        store.save(&doc).unwrap();
        assert_eq!(store.load("main").unwrap().queue, vec!["pending".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_schema_fails_loudly_naming_versions() {
        let root = tmp_root("schema");
        let dir = root.join(".lazydelta").join("threads").join("future");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("thread.json"),
            "{\"schema_version\":99,\"id\":\"future\"}",
        )
        .unwrap();
        let store = ThreadStore::new(&root);
        let (docs, errors) = store.load_all();
        assert!(docs.is_empty());
        assert_eq!(errors.len(), 1);
        let reason = &errors[0].reason;
        assert!(reason.contains("99") && reason.contains("1"), "reason: {reason}");
        assert!(errors[0].path.to_string_lossy().contains("thread.json"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn truncated_thread_does_not_block_others() {
        let root = tmp_root("mixed");
        let store = ThreadStore::new(&root);
        store.save(&sample("good")).unwrap();
        let dir = root.join(".lazydelta").join("threads").join("bad");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("thread.json"), "{\"schema_version\":1,\"id\":\"ba").unwrap();

        let (docs, errors) = store.load_all();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "good");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].reason.contains("invalid JSON"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
