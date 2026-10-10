//! lazydelta-core: the provenance-aware changeset model.
//!
//! The central idea: a changeset is an ordered list of hunks, each carrying a
//! provenance (`agent`, `human`, or `human`-fixed). Human edits made *while
//! reviewing* an agent hunk are recorded as [`HumanEdit`] entries, so a manual
//! fix is simultaneously (a) part of the changeset and (b) critique that can be
//! serialized back to the agent as feedback.
//!
//! Statuses model review mode: hunks start `Proposed`, and are gated by
//! `Accepted` / `Rejected` before they materialize into the worktree.

pub mod git;
pub mod snapshot;

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};

pub type HunkId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// One line of a hunk. `old_index` / `new_index` are 0-based line positions in
/// the base / proposed text respectively (absent for added / removed lines).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
}

/// Where the content of a hunk came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum Provenance {
    /// Produced by an agent in a given turn of a thread.
    Agent { agent: String, turn: u32 },
    /// Written directly by a human (no agent involvement).
    Human,
    /// Agent hunk that a human has edited during review.
    /// Details live in [`Hunk::human_edits`].
    HumanFix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HunkStatus {
    /// The default state: proposed, visible in the preview, not yet blessed.
    Proposed,
    /// Explicitly approved by the reviewer.
    Accepted,
    /// Rejected; its changes are reverted in the materialized output.
    Rejected,
}

/// A manual edit of an added line, made while reviewing the hunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HumanEdit {
    /// Index into [`Hunk::lines`].
    pub line: usize,
    /// The agent's original text.
    pub from: String,
    /// The human's replacement text (updated if the line is edited again).
    pub to: String,
}

/// A comment anchored to a hunk (`line: None`) or to one of its lines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub author: String,
    pub body: String,
    /// Index into [`Hunk::lines`] the comment is anchored to, if any.
    pub line: Option<usize>,
    pub replies: Vec<Comment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hunk {
    pub id: HunkId,
    /// 1-based, git-style ranges.
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
    pub lines: Vec<DiffLine>,
    pub provenance: Provenance,
    pub status: HunkStatus,
    pub human_edits: Vec<HumanEdit>,
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: PathBuf,
    /// Base text of the file (the "old" side).
    pub base: String,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializePolicy {
    /// Apply `Proposed` and `Accepted` hunks (working-tree preview).
    Proposed,
    /// Apply only `Accepted` hunks (what gets blessed / committed).
    AcceptedOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub files: Vec<FileChange>,
    next_hunk_id: HunkId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    HunkNotFound(HunkId),
    LineNotFound { hunk: HunkId, line: usize },
    NotAnAddedLine { hunk: HunkId, line: usize },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::HunkNotFound(id) => write!(f, "hunk {id} not found"),
            CoreError::LineNotFound { hunk, line } => write!(f, "line {line} not found in hunk {hunk}"),
            CoreError::NotAnAddedLine { hunk, line } => {
                write!(f, "line {line} of hunk {hunk} is not an added line")
            }
        }
    }
}

impl std::error::Error for CoreError {}

impl ChangeSet {
    /// Build a changeset from a base/proposed pair, attributing every hunk to
    /// `origin` (typically [`Provenance::Agent`]).
    pub fn from_diff(path: impl Into<PathBuf>, base: &str, proposed: &str, origin: Provenance) -> Self {
        let diff = TextDiff::from_lines(base, proposed);
        let mut hunks = Vec::new();
        let mut next_id: HunkId = 1;

        for group in diff.grouped_ops(3) {
            let mut lines: Vec<DiffLine> = Vec::new();
            let mut old_seen: Vec<usize> = Vec::new();
            let mut new_seen: Vec<usize> = Vec::new();

            for op in group.iter() {
                for change in diff.iter_changes(op) {
                let kind = match change.tag() {
                    ChangeTag::Equal => LineKind::Context,
                    ChangeTag::Delete => LineKind::Removed,
                    ChangeTag::Insert => LineKind::Added,
                };
                if let Some(i) = change.old_index() {
                    old_seen.push(i);
                }
                if let Some(i) = change.new_index() {
                    new_seen.push(i);
                }
                lines.push(DiffLine {
                    kind,
                    text: change.value().trim_end_matches('\n').to_string(),
                    old_index: change.old_index(),
                    new_index: change.new_index(),
                });
                }
            }

            let old_start = old_seen.iter().copied().min().map(|i| i + 1).unwrap_or(1);
            let old_len = old_seen.len();
            let new_start = new_seen.iter().copied().min().map(|i| i + 1).unwrap_or(1);
            let new_len = new_seen.len();

            hunks.push(Hunk {
                id: next_id,
                old_start,
                old_len,
                new_start,
                new_len,
                lines,
                provenance: origin.clone(),
                status: HunkStatus::Proposed,
                human_edits: Vec::new(),
                comments: Vec::new(),
            });
            next_id += 1;
        }

        ChangeSet {
            files: vec![FileChange {
                path: path.into(),
                base: base.to_string(),
                hunks,
            }],
            next_hunk_id: next_id,
        }
    }

    /// Build a changeset from base/current content maps (pure — no IO).
    /// Files present in `current` but not `base` are new (empty base). Files
    /// only in `base` (deletions) are skipped — v1 does not model deletions.
    pub fn from_contents(
        base: &BTreeMap<PathBuf, String>,
        current: &BTreeMap<PathBuf, String>,
        origin: Provenance,
    ) -> Self {
        let mut files = Vec::new();
        for (path, new_text) in current {
            let old_text = base.get(path).map(String::as_str).unwrap_or("");
            if old_text == new_text {
                continue;
            }
            let inner = ChangeSet::from_diff(path.clone(), old_text, new_text, origin.clone());
            files.push(FileChange {
                path: path.clone(),
                base: old_text.to_string(),
                hunks: inner.files.into_iter().next().map(|f| f.hunks).unwrap_or_default(),
            });
        }
        ChangeSet {
            next_hunk_id: files.iter().map(|f| f.hunks.len()).sum::<usize>() as HunkId + 1,
            files,
        }
    }

    /// Carry review state (statuses, comments, human edits) from an older
    /// changeset onto a freshly derived one, matching hunks on
    /// `(path, old_start, old_len)` (design D5). Returns human-readable
    /// warnings for dropped or ambiguous matches.
    pub fn carry_state_from(&mut self, old: &ChangeSet) -> Vec<String> {
        let mut warnings = Vec::new();
        for file in &mut self.files {
            let Some(old_file) = old.files.iter().find(|f| f.path == file.path) else {
                continue;
            };
            for hunk in &mut file.hunks {
                let matches: Vec<_> = old_file
                    .hunks
                    .iter()
                    .filter(|h| h.old_start == hunk.old_start && h.old_len == hunk.old_len)
                    .collect();
                match matches.len() {
                    0 => warnings.push(format!(
                        "{}: new hunk @{} has no prior review state",
                        file.path.display(),
                        hunk.old_start
                    )),
                    1 => {
                        let src = matches[0];
                        hunk.status = src.status;
                        hunk.comments = src.comments.clone();
                        // Re-record human edits that survived on disk.
                        hunk.human_edits.clear();
                        for edit in &src.human_edits {
                            let persisted = hunk.lines.iter().any(|l| {
                                l.kind == LineKind::Added && l.text == edit.to
                            });
                            if persisted {
                                hunk.human_edits.push(edit.clone());
                            }
                        }
                        if src.provenance == Provenance::HumanFix
                            && !hunk.human_edits.is_empty()
                        {
                            hunk.provenance = Provenance::HumanFix;
                        }
                    }
                    _ => {
                        let src = matches[0];
                        hunk.status = src.status;
                        warnings.push(format!(
                            "{}: ambiguous hunk match @{} — carried from the first prior hunk",
                            file.path.display(),
                            hunk.old_start
                        ));
                    }
                }
            }
        }
        warnings
    }

    fn file_mut(&mut self, hunk_id: HunkId) -> Option<&mut FileChange> {
        self.files.iter_mut().find(|f| f.hunks.iter().any(|h| h.id == hunk_id))
    }

    fn hunk_mut(&mut self, hunk_id: HunkId) -> Result<&mut Hunk, CoreError> {
        let file = self
            .file_mut(hunk_id)
            .ok_or(CoreError::HunkNotFound(hunk_id))?;
        file.hunks
            .iter_mut()
            .find(|h| h.id == hunk_id)
            .ok_or(CoreError::HunkNotFound(hunk_id))
    }

    pub fn hunk(&self, hunk_id: HunkId) -> Result<&Hunk, CoreError> {
        self.files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .find(|h| h.id == hunk_id)
            .ok_or(CoreError::HunkNotFound(hunk_id))
    }

    pub fn accept_hunk(&mut self, hunk_id: HunkId) -> Result<(), CoreError> {
        self.hunk_mut(hunk_id)?.status = HunkStatus::Accepted;
        Ok(())
    }

    pub fn reject_hunk(&mut self, hunk_id: HunkId) -> Result<(), CoreError> {
        self.hunk_mut(hunk_id)?.status = HunkStatus::Rejected;
        Ok(())
    }

    /// Return a hunk to `Proposed`.
    pub fn restore_hunk(&mut self, hunk_id: HunkId) -> Result<(), CoreError> {
        self.hunk_mut(hunk_id)?.status = HunkStatus::Proposed;
        Ok(())
    }

    /// Edit the text of an added line during review.
    ///
    /// Records a [`HumanEdit`] (preserving the agent's original text) and flips
    /// the hunk provenance to [`Provenance::HumanFix`] so the manual fix is a
    /// first-class review artifact, not just a silent file change.
    pub fn edit_added_line(
        &mut self,
        hunk_id: HunkId,
        line: usize,
        new_text: &str,
    ) -> Result<(), CoreError> {
        let hunk = self.hunk_mut(hunk_id)?;
        let dl = hunk
            .lines
            .get_mut(line)
            .ok_or(CoreError::LineNotFound { hunk: hunk_id, line })?;
        if dl.kind != LineKind::Added {
            return Err(CoreError::NotAnAddedLine { hunk: hunk_id, line });
        }
        if dl.text == new_text {
            return Ok(());
        }
        let original = hunk
            .human_edits
            .iter()
            .find(|e| e.line == line)
            .map(|e| e.from.clone())
            .unwrap_or_else(|| dl.text.clone());
        match hunk.human_edits.iter_mut().find(|e| e.line == line) {
            Some(edit) => edit.to = new_text.to_string(),
            None => hunk.human_edits.push(HumanEdit {
                line,
                from: original,
                to: new_text.to_string(),
            }),
        }
        dl.text = new_text.to_string();
        hunk.provenance = Provenance::HumanFix;
        Ok(())
    }

    pub fn add_comment(
        &mut self,
        hunk_id: HunkId,
        line: Option<usize>,
        author: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<(), CoreError> {
        let hunk = self.hunk_mut(hunk_id)?;
        hunk.comments.push(Comment {
            author: author.into(),
            body: body.into(),
            line,
            replies: Vec::new(),
        });
        Ok(())
    }

    /// Reconstruct full file contents from base + hunks, according to `policy`.
    ///
    /// Rejected hunks restore their base lines; accepted/proposed hunks emit
    /// their added lines (with any human edits applied). Human-authored
    /// rejection of individual *lines* is intentionally not modeled — review
    /// granularity is the hunk (see openspec change for per-line follow-up).
    pub fn materialize(&self, policy: MaterializePolicy) -> BTreeMap<PathBuf, String> {
        self.files
            .iter()
            .map(|file| (file.path.clone(), file.materialize(policy)))
            .collect()
    }

    /// Write the materialized files under `root` (creating directories).
    pub fn write_to(&self, root: &Path, policy: MaterializePolicy) -> std::io::Result<()> {
        for (path, contents) in self.materialize(policy) {
            let target = root.join(&path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, contents)?;
        }
        Ok(())
    }

    /// Serialize review feedback for the agent: every hunk that carries human
    /// edits or comments becomes a before/after pair or an anchored note.
    pub fn feedback_markdown(&self) -> String {
        let mut md = String::from("# Review feedback\n\n");
        let mut any = false;

        for file in &self.files {
            for hunk in &file.hunks {
                if hunk.human_edits.is_empty() && hunk.comments.is_empty() {
                    continue;
                }
                any = true;
                let _ = writeln!(
                    md,
                    "## `{}` — hunk {} (old lines {}–{}, {})",
                    file.path.display(),
                    hunk.id,
                    hunk.old_start,
                    hunk.old_start + hunk.old_len.saturating_sub(1),
                    match hunk.status {
                        HunkStatus::Proposed => "proposed",
                        HunkStatus::Accepted => "accepted",
                        HunkStatus::Rejected => "rejected",
                    }
                );
                if !hunk.human_edits.is_empty() {
                    let _ = writeln!(
                        md,
                        "\nI rewrote some of your lines while reviewing. Match this style in the rest of the change:"
                    );
                    for e in &hunk.human_edits {
                        let _ = writeln!(md, "- you wrote:  `{}`", e.from);
                        let _ = writeln!(md, "  I changed it to: `{}`", e.to);
                    }
                }
                if !hunk.comments.is_empty() {
                    let _ = writeln!(md, "\nComments:");
                    for c in &hunk.comments {
                        match c.line.and_then(|l| hunk.lines.get(l)) {
                            Some(dl) => {
                                let _ = writeln!(md, "- on `{}`: [{}] {}", dl.text, c.author, c.body);
                            }
                            None => {
                                let _ = writeln!(md, "- on the hunk: [{}] {}", c.author, c.body);
                            }
                        }
                    }
                }
                md.push('\n');
            }
        }

        if !any {
            md.push_str("No comments or manual edits yet. Review the hunks first.\n");
        }
        md
    }
}

impl FileChange {
    pub fn materialize(&self, policy: MaterializePolicy) -> String {
        let base: Vec<&str> = self.base.lines().collect();
        let mut out = String::new();
        let mut cursor = 0usize;

        let emit = |out: &mut String, text: &str| {
            out.push_str(text);
            out.push('\n');
        };

        for hunk in &self.hunks {
            let applies = match policy {
                MaterializePolicy::Proposed => hunk.status != HunkStatus::Rejected,
                MaterializePolicy::AcceptedOnly => hunk.status == HunkStatus::Accepted,
            };
            let first_old = hunk
                .lines
                .iter()
                .filter_map(|l| l.old_index)
                .min()
                .unwrap_or(cursor);
            while cursor < first_old.min(base.len()) {
                emit(&mut out, base[cursor]);
                cursor += 1;
            }
            for line in &hunk.lines {
                match line.kind {
                    LineKind::Context | LineKind::Removed => {
                        if let Some(oi) = line.old_index {
                            while cursor < oi.min(base.len()) {
                                emit(&mut out, base[cursor]);
                                cursor += 1;
                            }
                            cursor = oi + 1;
                            let keep_base = line.kind == LineKind::Context || !applies;
                            if keep_base {
                                emit(&mut out, &line.text);
                            }
                        }
                    }
                    LineKind::Added => {
                        if applies {
                            emit(&mut out, &line.text);
                        }
                    }
                }
            }
        }
        while cursor < base.len() {
            emit(&mut out, base[cursor]);
            cursor += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "fn main() {\n    let x = 1;\n    println!(\"{}\", x);\n}\n";
    const PROPOSED: &str = "fn main() {\n    let x = compute(1);\n    println!(\"{}\", x);\n    log::debug!(\"done\");\n}\n";

    fn demo() -> ChangeSet {
        ChangeSet::from_diff(
            "src/main.rs",
            BASE,
            PROPOSED,
            Provenance::Agent { agent: "claude-code".into(), turn: 1 },
        )
    }

    #[test]
    fn diff_produces_hunks_with_ranges_and_kinds() {
        let cs = demo();
        let hunks = &cs.files[0].hunks;
        assert!(!hunks.is_empty());
        assert!(hunks.iter().all(|h| h.status == HunkStatus::Proposed));
        let added: Vec<_> = hunks
            .iter()
            .flat_map(|h| h.lines.iter().filter(|l| l.kind == LineKind::Added))
            .collect();
        let removed: Vec<_> = hunks
            .iter()
            .flat_map(|h| h.lines.iter().filter(|l| l.kind == LineKind::Removed))
            .collect();
        assert_eq!(added.len(), 2); // `let x = compute(1);` and the new log line
        assert_eq!(removed.len(), 1);
        assert!(added.iter().any(|l| l.text.contains("log::debug")));
    }

    #[test]
    fn materialize_proposed_matches_proposed_text() {
        let cs = demo();
        let out = cs.materialize(MaterializePolicy::Proposed);
        let text = out.get(Path::new("src/main.rs")).unwrap();
        assert_eq!(text.lines().collect::<Vec<_>>(), PROPOSED.lines().collect::<Vec<_>>());
    }

    #[test]
    fn materialize_accepted_only_is_base_when_nothing_accepted() {
        let cs = demo();
        let out = cs.materialize(MaterializePolicy::AcceptedOnly);
        let text = out.get(Path::new("src/main.rs")).unwrap();
        assert_eq!(text.lines().collect::<Vec<_>>(), BASE.lines().collect::<Vec<_>>());
    }

    #[test]
    fn rejecting_a_hunk_restores_base_lines() {
        let mut cs = demo();
        let hunk_id = cs.files[0].hunks[0].id;
        let hunk2 = cs.hunk(hunk_id).unwrap().clone();
        cs.reject_hunk(hunk_id).unwrap();
        let text = {
            let out = cs.materialize(MaterializePolicy::Proposed);
            out.get(Path::new("src/main.rs")).unwrap().clone()
        };
        // The rejected hunk's additions are gone; the rest of the proposal stays.
        let rejected_texts: Vec<&str> = hunk2
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Added)
            .map(|l| l.text.as_str())
            .collect();
        for t in rejected_texts {
            assert!(!text.contains(t), "rejected line `{t}` should not appear");
        }
    }

    #[test]
    fn editing_an_added_line_records_human_fix() {
        let mut cs = demo();
        let hunk_id = cs.files[0].hunks[0].id;
        let (line_idx, original) = cs
            .hunk(hunk_id)
            .unwrap()
            .lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.kind == LineKind::Added)
            .map(|(i, l)| (i, l.text.clone()))
            .unwrap();
        let _ = original;

        cs.edit_added_line(hunk_id, line_idx, "    let x = compute(&input);").unwrap();

        let hunk = cs.hunk(hunk_id).unwrap();        assert_eq!(hunk.provenance, Provenance::HumanFix);
        assert_eq!(hunk.human_edits.len(), 1);
        assert_eq!(hunk.human_edits[0].to, "    let x = compute(&input);");
        assert_eq!(hunk.human_edits[0].from, original);
        let out = cs.materialize(MaterializePolicy::Proposed);
        let text = out.get(Path::new("src/main.rs")).unwrap();
        assert!(text.contains("compute(&input)"));
    }

    #[test]
    fn editing_a_context_line_is_rejected() {
        let mut cs = demo();
        let hunk_id = cs.files[0].hunks[0].id;
        let ctx = cs
            .hunk(hunk_id)
            .unwrap()
            .lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.kind == LineKind::Context)
            .map(|(i, _)| i)
            .unwrap();
        assert!(matches!(
            cs.edit_added_line(hunk_id, ctx, "nope"),
            Err(CoreError::NotAnAddedLine { .. })
        ));
    }

    #[test]
    fn feedback_markdown_contains_edits_and_comments() {
        let mut cs = demo();
        let hunk_id = cs.files[0].hunks[0].id;
        let line_idx = cs
            .hunk(hunk_id)
            .unwrap()
            .lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.kind == LineKind::Added)
            .map(|(i, _)| i)
            .unwrap();
        cs.edit_added_line(hunk_id, line_idx, "    let x = compute(&input);").unwrap();
        cs.add_comment(hunk_id, None, "you", "please keep side effects out").unwrap();

        let md = cs.feedback_markdown();
        assert!(md.contains("src/main.rs"));
        assert!(md.contains("I changed it to: `    let x = compute(&input);`"));
        assert!(md.contains("please keep side effects out"));
    }

    #[test]
    fn serde_roundtrip() {
        let mut cs = demo();
        let hunk_id = cs.files[0].hunks[0].id;
        let _ = cs.reject_hunk(hunk_id);
        let _ = cs.add_comment(hunk_id, Some(1), "you", "hm");
        let json = serde_json::to_string(&cs).unwrap();
        let back: ChangeSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cs);
    }

    #[test]
    fn carry_state_matches_hunks_by_key() {
        let mut old = demo();
        let hunk_id = old.files[0].hunks[0].id;
        let _ = old.accept_hunk(hunk_id);
        let _ = old.add_comment(hunk_id, None, "you", "keep this");

        // Fresh derivation of the same change: same hunk keys.
        let mut fresh = demo();
        let warnings = fresh.carry_state_from(&old);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(fresh.files[0].hunks[0].status, HunkStatus::Accepted);
        assert_eq!(fresh.files[0].hunks[0].comments.len(), 1);
    }

    #[test]
    fn carry_state_reports_no_match() {
        let old = demo();
        // A change in a region whose hunk keys cannot overlap the old ones.
        let mut fresh = ChangeSet::from_diff(
            "src/main.rs",
            "a\nb\nc\nd\ne\nf\ng\nh\n",
            "a\nb\nc\nd\ne\nf\nG\nh\n",
            Provenance::Agent { agent: "a".into(), turn: 2 },
        );
        let warnings = fresh.carry_state_from(&old);
        assert!(warnings.iter().any(|w| w.contains("no prior review state")));
    }

    #[test]
    fn carry_state_flags_ambiguity() {
        let old = demo();
        // Two old hunks with identical keys can't exist in one derivation,
        // but a manual construction simulates a split-after-refresh case.
        let mut split = old.clone();
        let orig = split.files[0].hunks[0].clone();
        let mut dup = orig.clone();
        dup.id = 99;
        split.files[0].hunks.push(dup);

        let mut fresh = demo();
        let warnings = fresh.carry_state_from(&split);
        assert!(warnings.iter().any(|w| w.contains("ambiguous")));
    }

    #[test]
    fn carry_state_keeps_only_persisted_human_edits() {
        let mut old = demo();
        let hunk_id = old.files[0].hunks[0].id;
        let line_idx = old
            .hunk(hunk_id)
            .unwrap()
            .lines
            .iter()
            .position(|l| l.kind == LineKind::Added)
            .unwrap();
        let _ = old.edit_added_line(hunk_id, line_idx, "    let x = compute(&input);");

        // Case 1: edit persisted to disk -> survives refresh.
        let persisted = ChangeSet::from_diff(
            "src/main.rs",
            BASE,
            &PROPOSED.replace("let x = compute(1);", "let x = compute(&input);"),
            Provenance::Agent { agent: "claude-code".into(), turn: 1 },
        );
        let mut p = persisted;
        let warnings = p.carry_state_from(&old);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!p.files[0].hunks[0].human_edits.is_empty());
        assert_eq!(p.files[0].hunks[0].provenance, Provenance::HumanFix);

        // Case 2: edit never hit disk (agent's text still there) -> dropped.
        let mut np = demo();
        let _ = np.carry_state_from(&old);
        assert!(np.files[0].hunks[0].human_edits.is_empty());
        assert_eq!(
            np.files[0].hunks[0].provenance,
            Provenance::Agent { agent: "claude-code".into(), turn: 1 }
        );
    }

    #[test]
    fn from_contents_skips_unchanged_and_deletions() {
        let mut base = BTreeMap::new();
        base.insert(PathBuf::from("a.rs"), "same\n".to_string());
        base.insert(PathBuf::from("gone.rs"), "deleted later\n".to_string());
        let mut current = BTreeMap::new();
        current.insert(PathBuf::from("a.rs"), "same\n".to_string());
        current.insert(PathBuf::from("new.rs"), "fresh\n".to_string());
        let cs = ChangeSet::from_contents(&base, &current, Provenance::Human);
        assert_eq!(cs.files.len(), 1);
        assert_eq!(cs.files[0].path, PathBuf::from("new.rs"));
        assert!(cs.files[0].hunks.iter().any(|h| h.lines.iter().any(|l| l.text == "fresh")));
    }

    #[test]
    fn write_to_materializes_files() {
        let mut cs = demo();
        let hunk_id = cs.files[0].hunks[0].id;
        let _ = cs.reject_hunk(hunk_id);
        let root = std::env::temp_dir().join(format!("lazydelta-test-{}", std::process::id()));
        cs.write_to(&root, MaterializePolicy::Proposed).unwrap();
        let written = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
        assert!(written.contains("fn main()"));
        std::fs::remove_dir_all(root).ok();
    }
}
