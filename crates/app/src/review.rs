//! The review surface: renders a [`ChangeSet`] as hunks with provenance,
//! per-hunk accept/reject (leader menu), editable added lines (insert mode),
//! anchored comments, the agent thread panel, and the external-editor jump.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    App, AsyncApp, Context, Entity, FocusHandle, Focusable, MouseButton, Rgba, SharedString,
    Window, actions, div, prelude::*, px, rgb, rgba,
};
use lazydelta_app::config::{self, Config};
use lazydelta_app::markdown::render_markdown;
use lazydelta_app::project::Project;
use lazydelta_app::runner::{RunnerEvent, RunnerHandle, spawn_runner};
use lazydelta_app::store::{AgentMessage, ThreadDocument, ThreadStore};
use lazydelta_core::{ChangeSet, HunkId, HunkStatus, LineKind, MaterializePolicy, Provenance};

use crate::editable_line::{EditEvent, EditableLine};

actions!(
    review,
    [
        // NORMAL-mode navigation and mode keys
        NavDown,
        NavUp,
        LastLine,
        PendingG,
        PendingBracketFwd,
        PendingBracketBack,
        LeaderKey,
        InsertStart,
        JumpToEditor,
        RefreshChangeset,
        ConfirmYes,
        ConfirmNo,
        CycleAgent,
        DismissNotice,
        // PENDING-sequence continuations
        FirstLine,
        HKey,
        // LEADER-menu actions
        AcceptHunk,
        RejectHunk,
        RestoreHunk,
        CommentOnLine,
        SendFeedback,
        EditorJump,
        WriteBackPreview,
        WriteBackAccepted,
        ToggleThread,
        RestartSession,
    ]
);

fn bg() -> Rgba { rgb(0x16161e) }
fn card() -> Rgba { rgb(0x1d1d28) }
fn card_rejected() -> Rgba { rgb(0x1a171c) }
fn added_bg() -> Rgba { rgba(0x9ece6a20) }
fn removed_bg() -> Rgba { rgba(0xf7768e1a) }
fn focus_bg() -> Rgba { rgba(0x7aa2f726) }
fn green() -> Rgba { rgb(0x9ece6a) }
fn red() -> Rgba { rgb(0xf7768e) }
fn blue() -> Rgba { rgb(0x7aa2f7) }
fn yellow() -> Rgba { rgb(0xe0af68) }
fn grey() -> Rgba { rgb(0x565f89) }
fn text_col() -> Rgba { rgb(0xc0caf5) }

/// A pending two-key sequence: `g` (waiting `g`), `]`/`[` (waiting `h`),
/// or ` ` (the leader menu).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pending {
    G,
    BracketFwd,
    BracketBack,
    Leader,
}

impl Pending {
    fn label(&self) -> &'static str {
        match self {
            Pending::G => "g…",
            Pending::BracketFwd => "]…",
            Pending::BracketBack => "[…",
            Pending::Leader => "space _",
        }
    }
}

/// Confirmation dialog: write the preview (or accepted-only) state to the
/// working tree, optionally jumping to an external editor afterwards.
enum Confirm {
    WriteBack {
        accepted_only: bool,
        then_jump: Option<(PathBuf, usize)>,
    },
}

struct ProjectState {
    project: Project,
    config: Config,
    runner: Option<RunnerHandle>,
    thread_id: String,
    agent_index: usize,
}

impl ProjectState {
    fn current_agent(&self) -> Option<&config::AgentConfig> {
        self.config.agents.get(self.agent_index)
    }

    fn spawn_current_runner(&mut self) -> Option<RunnerHandle> {
        self.current_agent().cloned().map(|agent| {
            spawn_runner(
                agent,
                self.project.root.clone(),
                Duration::from_secs(self.config.prompt_timeout_secs),
            )
        })
    }
}

pub struct ReviewView {
    pub focus_handle: FocusHandle,
    pub changeset: ChangeSet,
    focused: Option<(HunkId, usize)>,
    editing: Option<(HunkId, usize, Entity<EditableLine>)>,
    comment_target: Option<(HunkId, usize)>,
    comment_draft: Option<Entity<EditableLine>>,
    feedback_preview: Option<SharedString>,
    pending: Option<Pending>,
    hint: Option<String>,
    error_banner: Option<String>,
    messages: Vec<AgentMessage>,
    collapsed: bool,
    queue_sent: usize,
    agent_name: Option<String>,
    confirm: Option<Confirm>,
    project: Option<ProjectState>,
}

impl ReviewView {
    pub fn new(changeset: ChangeSet, cx: &mut Context<Self>) -> Self {
        let focused = first_added_line(&changeset);
        ReviewView {
            focus_handle: cx.focus_handle(),
            changeset,
            focused,
            editing: None,
            comment_target: None,
            comment_draft: None,
            feedback_preview: None,
            pending: None,
            hint: None,
            error_banner: None,
            messages: Vec::new(),
            collapsed: false,
            queue_sent: 0,
            agent_name: None,
            confirm: None,
            project: None,
        }
    }

    /// Attach a project (persistence + agent runner) to this review.
    pub fn attach_project(
        &mut self,
        project: Project,
        config: Config,
        thread_id: String,
        persisted: Option<ThreadDocument>,
        cx: &mut Context<Self>,
    ) {
        if let Some(doc) = persisted {
            self.messages = doc.messages;
            self.collapsed = doc.collapsed;
        }
        let mut state = ProjectState {
            project,
            config,
            runner: None,
            thread_id,
            agent_index: 0,
        };
        if let Some(runner) = state.spawn_current_runner() {
            self.agent_name = state.current_agent().map(|a| a.name.clone());
            poll_runner(runner.rx.clone(), cx);
            state.runner = Some(runner);
        }
        self.project = Some(state);
        self.save(cx);
    }

    // ---- Mode / keymap state ----

    fn mode_label(&self) -> SharedString {
        if self.editing.is_some() || self.comment_draft.is_some() {
            "INSERT".into()
        } else {
            "NORMAL".into()
        }
    }

    fn key_context(&self) -> &'static str {
        if self.editing.is_some() || self.comment_draft.is_some() {
            "ReviewInsert"
        } else {
            match self.pending {
                Some(Pending::Leader) => "ReviewLeader",
                Some(_) => "ReviewPending",
                None => "ReviewNormal",
            }
        }
    }

    fn clear_pending(&mut self, why: &str) {
        if self.pending.take().is_some() {
            self.hint = Some(why.to_string());
        }
    }

    fn enter_pending(&mut self, pending: Pending, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            self.clear_pending("sequence cancelled");
        } else {
            self.pending = Some(pending);
            arm_timeout(pending, cx);
        }
        cx.notify();
    }

    /// Runs when a pending-sequence continuation arrives. If the pending
    /// state does not match, the sequence is cancelled and nothing runs.
    fn continue_sequence(
        &mut self,
        want: Pending,
        run: impl FnOnce(&mut Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) {
        if self.pending == Some(want) {
            self.pending = None;
            run(self, cx);
        } else {
            self.clear_pending("sequence cancelled");
            cx.notify();
        }
    }

    // ---- Navigation ----

    fn added_line_positions(&self) -> Vec<(HunkId, usize)> {
        self.changeset
            .files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .flat_map(|h| {
                h.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.kind == LineKind::Added)
                    .map(move |(i, _)| (h.id, i))
            })
            .collect()
    }

    fn hunk_order(&self) -> Vec<HunkId> {
        self.changeset
            .files
            .iter()
            .flat_map(|f| f.hunks.iter().map(|h| h.id))
            .collect()
    }

    fn nav_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let positions = self.added_line_positions();
        if positions.is_empty() {
            self.hint = Some("no added lines to focus".into());
            cx.notify();
            return;
        }
        let idx = self
            .focused
            .and_then(|cur| positions.iter().position(|p| *p == cur));
        let next = match (forward, idx) {
            (true, Some(i)) if i + 1 >= positions.len() => {
                self.hint = Some("last added line".into());
                i
            }
            (false, Some(0)) | (false, None) => {
                self.hint = Some("first added line".into());
                0
            }
            (true, Some(i)) => i + 1,
            (false, Some(i)) => i - 1,
            (true, None) => 0,
        };
        self.focused = Some(positions[next]);
        cx.notify();
    }

    fn first_line(&mut self, cx: &mut Context<Self>) {
        if let Some(first) = first_added_line(&self.changeset) {
            self.focused = Some(first);
        } else {
            self.hint = Some("no added lines to focus".into());
        }
        cx.notify();
    }

    fn last_line(&mut self, cx: &mut Context<Self>) {
        let positions = self.added_line_positions();
        if let Some(last) = positions.last() {
            self.focused = Some(*last);
        } else {
            self.hint = Some("no added lines to focus".into());
        }
        cx.notify();
    }

    fn hunk_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let order = self.hunk_order();
        let idx = self
            .focused
            .map(|(h, _)| h)
            .and_then(|h| order.iter().position(|o| *o == h));
        let target = match (forward, idx) {
            (true, Some(i)) if i + 1 >= order.len() => {
                self.hint = Some("no next hunk".into());
                None
            }
            (false, Some(0)) | (false, None) => {
                self.hint = Some("no previous hunk".into());
                None
            }
            (true, Some(i)) => Some(i + 1),
            (false, Some(i)) => Some(i - 1),
            (true, None) => (!order.is_empty()).then_some(0),
        };
        if let Some(t) = target {
            let hunk_id = order[t];
            let first_added = self
                .changeset
                .hunk(hunk_id)
                .ok()
                .and_then(|h| h.lines.iter().position(|l| l.kind == LineKind::Added));
            self.focused = first_added.map(|i| (hunk_id, i));
        }
        cx.notify();
    }

    // ---- Leader-gated review actions ----

    /// Run `f` when a leader sequence is being completed; a bare press of the
    /// key in NORMAL mode is a no-op (actions live in the space menu, D8).
    fn leader(&mut self, f: impl FnOnce(&mut Self, &mut Context<Self>), cx: &mut Context<Self>) {
        match self.pending.take() {
            Some(Pending::Leader) => f(self, cx),
            Some(_) => {
                self.hint = Some("sequence cancelled".into());
                cx.notify();
            }
            None => {
                self.hint = Some("press space first (leader menu)".into());
                cx.notify();
            }
        }
    }

    fn accept(&mut self, cx: &mut Context<Self>) {
        self.leader(|this, cx| {
            if let Some(h) = this.focused.map(|(h, _)| h) {
                let _ = this.changeset.accept_hunk(h);
                this.save(cx);
                cx.notify();
            }
        }, cx);
    }

    fn reject(&mut self, cx: &mut Context<Self>) {
        self.leader(|this, cx| {
            if let Some(h) = this.focused.map(|(h, _)| h) {
                let _ = this.changeset.reject_hunk(h);
                this.save(cx);
                cx.notify();
            }
        }, cx);
    }

    fn restore(&mut self, cx: &mut Context<Self>) {
        self.leader(|this, cx| {
            if let Some(h) = this.focused.map(|(h, _)| h) {
                let _ = this.changeset.restore_hunk(h);
                this.save(cx);
                cx.notify();
            }
        }, cx);
    }

    fn start_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            self.clear_pending("sequence cancelled");
            cx.notify();
            return;
        }
        let Some((hunk_id, line)) = self.focused else {
            self.hint = Some("focus an added line first".into());
            cx.notify();
            return;
        };
        let Ok(hunk) = self.changeset.hunk(hunk_id) else {
            return;
        };
        if hunk.lines[line].kind != LineKind::Added {
            self.hint = Some("only added lines are editable (i/a)".into());
            cx.notify();
            return;
        }
        let text = hunk.lines[line].text.clone();
        let return_focus = self.focus_handle.clone();
        let entity = cx.new(|cx| EditableLine::new(&text, return_focus, true, cx));
        let handle = entity.read(cx).focus_handle.clone();
        cx.subscribe(&entity, |this, _, event, cx| match event {
            EditEvent::Commit(text) => this.finish_edit(text.clone(), cx),
            EditEvent::Cancel => {
                this.editing = None;
                cx.notify();
            }
        })
        .detach();
        self.editing = Some((hunk_id, line, entity));
        window.focus(&handle);
        cx.notify();
    }

    fn finish_edit(&mut self, text: String, cx: &mut Context<Self>) {
        if let Some((hunk_id, line, _)) = self.editing.take() {
            let _ = self.changeset.edit_added_line(hunk_id, line, &text);
            self.focused = Some((hunk_id, line));
            self.save(cx);
        }
        cx.notify();
    }

    fn start_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leader(
            |this, cx| {
                let Some(target) = this.focused else { return };
                let return_focus = this.focus_handle.clone();
                let entity = cx.new(|cx| EditableLine::new("", return_focus, false, cx));
                let handle = entity.read(cx).focus_handle.clone();
                cx.subscribe(&entity, |this, _, event, cx| match event {
                    EditEvent::Commit(text) => this.finish_comment(text.clone(), cx),
                    EditEvent::Cancel => {
                        this.comment_draft = None;
                        this.comment_target = None;
                        cx.notify();
                    }
                })
                .detach();
                this.comment_target = Some(target);
                this.comment_draft = Some(entity.clone());
                window.focus(&handle);
                cx.notify();
            },
            cx,
        );
    }

    fn finish_comment(&mut self, text: String, cx: &mut Context<Self>) {
        if let (Some((hunk_id, line)), Some(_)) =
            (self.comment_target.take(), self.comment_draft.take())
            && !text.trim().is_empty()
        {
            let _ = self.changeset.add_comment(hunk_id, Some(line), "you", text.trim());
            self.save(cx);
        }
        cx.notify();
    }

    // ---- Feedback loop ----

    fn send_feedback(&mut self, cx: &mut Context<Self>) {
        self.leader(
            |this, cx| {
                let all = this
                    .changeset
                    .files
                    .iter()
                    .flat_map(|f| f.hunks.iter())
                    .count();
                let accepted = this
                    .changeset
                    .files
                    .iter()
                    .flat_map(|f| f.hunks.iter())
                    .filter(|h| h.status == HunkStatus::Accepted)
                    .count();
                let rejected = this
                    .changeset
                    .files
                    .iter()
                    .flat_map(|f| f.hunks.iter())
                    .filter(|h| h.status == HunkStatus::Rejected)
                    .count();
                let prompt = format!(
                    "# Thread summary\n\n{all} hunks: {accepted} accepted, {rejected} rejected, {} proposed.\n\n{}",
                    all - accepted - rejected,
                    this.changeset.feedback_markdown()
                );
                let Some(state) = this.project.as_ref() else {
                    this.error_banner = Some(
                        "no project/agent: start with --project <dir> and configure .lazydelta/config.json"
                            .into(),
                    );
                    cx.notify();
                    return;
                };
                let Some(runner) = state.runner.as_ref() else {
                    this.error_banner = Some(format!(
                        "no agent configured in {}",
                        Config::config_path(&state.project.root).display()
                    ));
                    cx.notify();
                    return;
                };
                runner.prompt(prompt.clone());
                this.queue_sent += 1;
                this.feedback_preview = Some(prompt.clone().into());
                this.messages.push(AgentMessage {
                    role: "you".into(),
                    text: prompt,
                });
                this.save(cx);
                cx.notify();
            },
            cx,
        );
    }

    fn on_runner_event(&mut self, event: RunnerEvent, cx: &mut Context<Self>) {
        match event {
            RunnerEvent::SessionStarted { agent, .. } => {
                self.agent_name = Some(agent);
                self.hint = Some("agent session started".into());
            }
            RunnerEvent::Message { text } => {
                match self.messages.last_mut() {
                    Some(last) if last.role == "agent" => last.text.push_str(&text),
                    _ => self.messages.push(AgentMessage {
                        role: "agent".into(),
                        text,
                    }),
                }
                self.save(cx);
            }
            RunnerEvent::PromptDone => {
                self.queue_sent = self.queue_sent.saturating_sub(1);
                self.save(cx);
            }
            RunnerEvent::PromptError { error } => {
                self.queue_sent = self.queue_sent.saturating_sub(1);
                self.error_banner = Some(format!("{error} — [space x] restart session"));
            }
            RunnerEvent::Fatal { error } => {
                if let Some(state) = self.project.as_mut() {
                    state.runner = None;
                }
                self.error_banner =
                    Some(format!("{error} — fix config, then [space x] restart session"));
            }
        }
        cx.notify();
    }

    // ---- Write-back, jump, refresh, misc ----

    fn write_back(&mut self, accepted_only: bool, cx: &mut Context<Self>) {
        self.leader(
            |this, cx| {
                let Some(state) = this.project.as_ref() else {
                    this.hint = Some("write-back needs a project".into());
                    cx.notify();
                    return;
                };
                let policy = if accepted_only {
                    MaterializePolicy::AcceptedOnly
                } else {
                    MaterializePolicy::Proposed
                };
                let differs = this
                    .changeset
                    .materialize(policy)
                    .into_iter()
                    .any(|(path, preview)| {
                        !state.project.disk_matches_preview(&this.changeset, &path, &preview)
                    });
                if differs {
                    this.confirm = Some(Confirm::WriteBack {
                        accepted_only,
                        then_jump: None,
                    });
                    this.hint = Some("working tree differs — write it? [y/n]".into());
                } else {
                    let _ = this.changeset.write_to(&state.project.root, policy);
                    this.hint = Some(
                        if accepted_only { "accepted state written" } else { "preview written" }
                            .into(),
                    );
                }
                cx.notify();
            },
            cx,
        );
    }

    fn jump_to_editor(&mut self, via_leader: bool, cx: &mut Context<Self>) {
        fn run(this: &mut ReviewView, cx: &mut Context<ReviewView>) {
            let Some(state) = this.project.as_ref() else {
                this.hint = Some("editor jump needs a project".into());
                cx.notify();
                return;
            };
            let Some(editor) = state.config.effective_editor() else {
                this.error_banner = Some(format!(
                    "no editor configured — set \"editor\" or \"editor_preset\" in {}",
                    Config::config_path(&state.project.root).display()
                ));
                cx.notify();
                return;
            };
            let Some((hunk_id, line_idx)) = this.focused else {
                this.hint = Some("focus a line first".into());
                cx.notify();
                return;
            };
            let Ok(hunk) = this.changeset.hunk(hunk_id) else { return };
            let line = hunk
                .lines
                .get(line_idx)
                .and_then(|l| l.new_index)
                .map(|i| i + 1)
                .unwrap_or(hunk.new_start);
            let Some(file) = this
                .changeset
                .files
                .iter()
                .find(|f| f.hunks.iter().any(|h| h.id == hunk_id))
                .map(|f| f.path.clone())
            else {
                return;
            };
            let preview = this
                .changeset
                .materialize(MaterializePolicy::Proposed)
                .get(&file)
                .cloned()
                .unwrap_or_default();
            if state.project.disk_matches_preview(&this.changeset, &file, &preview) {
                let abs = state.project.root.join(&file);
                match config::launch_editor(&editor, &abs, line) {
                    Ok(()) => {
                        this.hint = Some(format!("opened {} at line {line}", editor.command))
                    }
                    Err(e) => {
                        this.error_banner =
                            Some(format!("cannot launch {}: {e}", editor.command))
                    }
                }
            } else {
                this.confirm = Some(Confirm::WriteBack {
                    accepted_only: false,
                    then_jump: Some((file, line)),
                });
                this.hint =
                    Some("working tree differs from preview — write back first? [y/n]".into());
            }
            cx.notify();
        }
        if via_leader {
            self.leader(run, cx);
        } else if self.pending.is_some() {
            self.clear_pending("sequence cancelled");
            cx.notify();
        } else {
            run(self, cx);
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            self.clear_pending("sequence cancelled");
            cx.notify();
            return;
        }
        let Some(state) = self.project.as_ref() else {
            self.hint = Some("refresh needs a project".into());
            cx.notify();
            return;
        };
        match state.project.refresh(&self.changeset) {
            Ok((fresh, warnings)) => {
                self.changeset = fresh;
                if let Some((h, _)) = self.focused {
                    if self.changeset.hunk(h).is_err() {
                        self.focused = first_added_line(&self.changeset);
                    }
                } else {
                    self.focused = first_added_line(&self.changeset);
                }
                self.hint = Some(
                    warnings
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "refreshed".into()),
                );
                self.save(cx);
                cx.notify();
            }
            Err(e) => {
                self.error_banner = Some(format!("refresh failed: {e}"));
                cx.notify();
            }
        }
    }

    fn restart_session(&mut self, cx: &mut Context<Self>) {
        self.leader(
            |this, cx| {
                let Some(state) = this.project.as_mut() else { return };
                if let Some(runner) = state.runner.take() {
                    runner.stop();
                }
                this.error_banner = None;
                if let Some(runner) = state.spawn_current_runner() {
                    this.agent_name = state.current_agent().map(|a| a.name.clone());
                    poll_runner(runner.rx.clone(), cx);
                    state.runner = Some(runner);
                    this.hint = Some("restarting session".into());
                } else {
                    this.error_banner = Some("no agent configured".into());
                }
                cx.notify();
            },
            cx,
        );
    }

    fn cycle_agent(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            self.clear_pending("sequence cancelled");
            cx.notify();
            return;
        }
        let Some(state) = self.project.as_mut() else {
            return;
        };
        if state.config.agents.len() < 2 {
            self.hint = Some("only one (or no) agent configured".into());
            cx.notify();
            return;
        }
        state.agent_index = (state.agent_index + 1) % state.config.agents.len();
        if let Some(runner) = state.runner.take() {
            runner.stop();
        }
        let new_runner = state.spawn_current_runner();
        self.agent_name = state.current_agent().map(|a| a.name.clone());
        self.hint = Some(format!(
            "agent: {} (of {})",
            self.agent_name.clone().unwrap_or_default(),
            state.config.agents.len()
        ));
        if let Some(runner) = new_runner {
            poll_runner(runner.rx.clone(), cx);
            state.runner = Some(runner);
        }
        cx.notify();
    }

    fn toggle_thread(&mut self, cx: &mut Context<Self>) {
        self.leader(
            |this, cx| {
                this.collapsed = !this.collapsed;
                this.save(cx);
                cx.notify();
            },
            cx,
        );
    }

    fn confirm_yes(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            self.clear_pending("sequence cancelled");
            cx.notify();
            return;
        }
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        match confirm {
            Confirm::WriteBack { accepted_only, then_jump } => {
                let Some(state) = self.project.as_ref() else { return };
                let policy = if accepted_only {
                    MaterializePolicy::AcceptedOnly
                } else {
                    MaterializePolicy::Proposed
                };
                match self.changeset.write_to(&state.project.root, policy) {
                    Ok(()) => self.hint = Some("written to working tree".into()),
                    Err(e) => {
                        self.error_banner = Some(format!("write-back failed: {e}"));
                        self.hint = Some("write-back failed".into());
                    }
                }
                if let Some((file, line)) = then_jump
                    && let Some(editor) = state.config.effective_editor()
                {
                    let abs = state.project.root.join(&file);
                    match config::launch_editor(&editor, &abs, line) {
                        Ok(()) => {
                            self.hint = Some(format!("opened {} at line {line}", editor.command))
                        }
                        Err(e) => {
                            self.error_banner =
                                Some(format!("cannot launch {}: {e}", editor.command))
                        }
                    }
                }
            }
        }
        cx.notify();
    }

    fn confirm_no(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            self.clear_pending("sequence cancelled");
            cx.notify();
            return;
        }
        if self.confirm.take().is_some() {
            self.hint = Some("cancelled".into());
            cx.notify();
        }
    }

    fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        if self.pending.take().is_some() {
            changed = true;
        }
        if self.error_banner.take().is_some() {
            changed = true;
        }
        if self.hint.take().is_some() {
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    // ---- Persistence ----

    fn save(&self, _cx: &Context<Self>) {
        let Some(state) = self.project.as_ref() else { return };
        let doc = ThreadDocument {
            schema_version: lazydelta_app::store::SCHEMA_VERSION,
            id: state.thread_id.clone(),
            changeset: self.changeset.clone(),
            messages: self.messages.clone(),
            queue: Vec::new(),
            collapsed: self.collapsed,
        };
        let store = ThreadStore::new(&state.project.root);
        if let Err(e) = store.save(&doc) {
            eprintln!("lazydelta: cannot save thread: {e}");
        }
    }

    // ---- Rendering helpers ----

    fn provenance_badge(provenance: &Provenance) -> (&'static str, Rgba) {
        match provenance {
            Provenance::Agent { .. } => ("AGENT", blue()),
            Provenance::Human => ("HUMAN", yellow()),
            Provenance::HumanFix => ("HUMAN FIX", yellow()),
        }
    }

    fn status_chip(status: HunkStatus) -> (&'static str, Rgba) {
        match status {
            HunkStatus::Proposed => ("proposed", grey()),
            HunkStatus::Accepted => ("accepted", green()),
            HunkStatus::Rejected => ("rejected", red()),
        }
    }

    fn line_row(
        &self,
        hunk_id: HunkId,
        line_idx: usize,
        line: &lazydelta_core::DiffLine,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let is_focused = self.focused == Some((hunk_id, line_idx));
        let is_editing = matches!(self.editing, Some((h, l, _)) if h == hunk_id && l == line_idx);

        let (marker, marker_color, line_bg) = match line.kind {
            LineKind::Context => (" ", grey(), None),
            LineKind::Added => ("+", green(), Some(added_bg())),
            LineKind::Removed => ("-", red(), Some(removed_bg())),
        };

        let mut row = div()
            .id(("line", hunk_id as u32 * 1000 + line_idx as u32))
            .flex()
            .flex_row();
        if is_focused {
            row = row.bg(focus_bg()).border_l_2().border_color(blue());
        } else {
            row = row.border_l_2().border_color(gpui::transparent_black());
            if let Some(lbg) = line_bg {
                row = row.bg(lbg);
            }
        }

        let content: gpui::AnyElement = if is_editing {
            let (_, _, entity) = self.editing.as_ref().unwrap();
            entity.clone().into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .child(line.text.clone())
                .into_any_element()
        };

        row.child(
            div()
                .w(px(28.))
                .flex_none()
                .text_color(marker_color)
                .child(marker),
        )
        .child(content)
        .when(
            is_focused && line.kind == LineKind::Added && !is_editing,
            |row| {
                row.child(
                    div()
                        .flex_none()
                        .pr_2()
                        .text_size(px(11.))
                        .text_color(grey())
                        .child("i insert · space c comment · space e editor"),
                )
            },
        )
        .when(line.kind == LineKind::Added && !is_editing, |row| {
            row.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &gpui::MouseDownEvent, window, cx| {
                    this.focused = Some((hunk_id, line_idx));
                    let handle = this.focus_handle.clone();
                    window.focus(&handle);
                    cx.notify();
                }),
            )
        })
    }

    fn line_and_comments(
        &self,
        hunk_id: HunkId,
        line_idx: usize,
        line: &lazydelta_core::DiffLine,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let comments: Vec<_> = self
            .changeset
            .files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .find(|h| h.id == hunk_id)
            .map(|h| {
                h.comments
                    .iter()
                    .filter(|c| c.line == Some(line_idx))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let row = self.line_row(hunk_id, line_idx, line, cx);

        div().flex().flex_col().child(row).children(
            comments
                .into_iter()
                .map(|c| {
                    div()
                        .ml_9()
                        .mr_2()
                        .my_1()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(rgba(0x7aa2f71a))
                        .border_1()
                        .border_color(rgb(0x33467c))
                        .text_size(px(12.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_color(blue())
                                        .child(format!("{} commented:", c.author)),
                                )
                                .child(div().text_color(text_col()).child(c.body.clone())),
                        )
                }),
        )
    }

    fn hunk_card(
        &self,
        hunk: &lazydelta_core::Hunk,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let (badge, badge_color) = Self::provenance_badge(&hunk.provenance);
        let (status, status_color) = Self::status_chip(hunk.status);
        let card_bg = if hunk.status == HunkStatus::Rejected {
            card_rejected()
        } else {
            card()
        };

        div()
            .id(("hunk", hunk.id as u32))
            .bg(card_bg)
            .border_1()
            .border_color(rgb(0x2a2a3a))
            .rounded_md()
            .p_2()
            .mb_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .px_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(badge_color)
                            .text_color(badge_color)
                            .text_size(px(10.))
                            .child(badge),
                    )
                    .child(
                        div()
                            .px_1()
                            .rounded_sm()
                            .text_color(status_color)
                            .text_size(px(10.))
                            .child(status),
                    )
                    .child(
                        div()
                            .text_color(grey())
                            .text_size(px(10.))
                            .child(format!(
                                "@{},{} → {},{}",
                                hunk.old_start,
                                hunk.old_start + hunk.old_len.saturating_sub(1),
                                hunk.new_start,
                                hunk.new_start + hunk.new_len.saturating_sub(1),
                            )),
                    )
                    .when(!hunk.human_edits.is_empty(), |el| {
                        el.child(
                            div()
                                .text_color(yellow())
                                .text_size(px(10.))
                                .child(format!("{} edited", hunk.human_edits.len())),
                        )
                    })
                    .when(!hunk.comments.is_empty(), |el| {
                        el.child(
                            div()
                                .text_color(blue())
                                .text_size(px(10.))
                                .child(format!("{} 💬", hunk.comments.len())),
                        )
                    }),
            )
            .children(
                hunk.lines
                    .iter()
                    .enumerate()
                    .map(|(i, line)| self.line_and_comments(hunk.id, i, line, cx)),
            )
    }

    fn stats(&self) -> (usize, usize, usize, usize) {
        self.changeset
            .files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .fold((0, 0, 0, 0), |mut acc, h| {
                acc.0 += 1;
                match h.status {
                    HunkStatus::Proposed => acc.1 += 1,
                    HunkStatus::Accepted => acc.2 += 1,
                    HunkStatus::Rejected => acc.3 += 1,
                }
                acc
            })
    }
}

fn first_added_line(cs: &ChangeSet) -> Option<(HunkId, usize)> {
    cs.files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .find_map(|h| {
            h.lines
                .iter()
                .position(|l| l.kind == LineKind::Added)
                .map(|i| (h.id, i))
        })
}

/// 1s timeout for pending sequences (tasks 6.1/6.2).
fn arm_timeout(pending: Pending, cx: &mut Context<ReviewView>) {
    cx.spawn(async move |this: gpui::WeakEntity<ReviewView>, cx: &mut AsyncApp| {
        cx.background_executor()
            .timer(Duration::from_secs(1))
            .await;
        let _ = this.update(cx, |v, cx| {
            if v.pending == Some(pending) {
                v.pending = None;
                v.hint = Some("sequence timed out".into());
                cx.notify();
            }
        });
    })
    .detach();
}

/// Poll the runner's event channel on a UI timer (design D1).
fn poll_runner(rx: Arc<Mutex<Receiver<RunnerEvent>>>, cx: &mut Context<ReviewView>) {
    cx.spawn(async move |this: gpui::WeakEntity<ReviewView>, cx: &mut AsyncApp| {
        loop {
            loop {
                let event = rx.lock().expect("runner receiver poisoned").try_recv();
                match event {
                    Ok(event) => {
                        // A dead view means the window is gone; stop polling.
                        if this.update(cx, |v, cx| v.on_runner_event(event, cx)).is_err() {
                            return;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            }
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
        }
    })
    .detach();
}

impl Focusable for ReviewView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ReviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stats = self.stats();
        let key_context = self.key_context();
        let mode = self.mode_label();
        let insert = mode == "INSERT";

        div()
            .id("review-root")
            .key_context(key_context)
            .track_focus(&self.focus_handle(cx))
            .font_family("JetBrains Mono")
            .text_size(px(13.))
            .text_color(text_col())
            .bg(bg())
            .size_full()
            .flex()
            .flex_col()
            // NORMAL-mode bindings
            .on_action(cx.listener(|this, _: &NavDown, _, cx| this.nav_step(true, cx)))
            .on_action(cx.listener(|this, _: &NavUp, _, cx| this.nav_step(false, cx)))
            .on_action(cx.listener(|this, _: &LastLine, _, cx| this.last_line(cx)))
            .on_action(cx.listener(|this, _: &PendingG, _, cx| {
                this.enter_pending(Pending::G, cx)
            }))
            .on_action(cx.listener(|this, _: &PendingBracketFwd, _, cx| {
                this.enter_pending(Pending::BracketFwd, cx)
            }))
            .on_action(cx.listener(|this, _: &PendingBracketBack, _, cx| {
                this.enter_pending(Pending::BracketBack, cx)
            }))
            .on_action(cx.listener(|this, _: &LeaderKey, _, cx| {
                this.enter_pending(Pending::Leader, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertStart, window, cx| {
                this.start_edit(window, cx)
            }))
            .on_action(cx.listener(|this, _: &JumpToEditor, _, cx| {
                this.jump_to_editor(false, cx)
            }))
            .on_action(cx.listener(|this, _: &RefreshChangeset, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &ConfirmYes, _, cx| this.confirm_yes(cx)))
            .on_action(cx.listener(|this, _: &ConfirmNo, _, cx| this.confirm_no(cx)))
            .on_action(cx.listener(|this, _: &CycleAgent, _, cx| this.cycle_agent(cx)))
            .on_action(cx.listener(|this, _: &DismissNotice, _, cx| this.dismiss_notice(cx)))
            // PENDING-sequence continuations
            .on_action(cx.listener(|this, _: &FirstLine, _, cx| {
                this.continue_sequence(Pending::G, ReviewView::first_line, cx)
            }))
            .on_action(cx.listener(|this, _: &HKey, _, cx| match this.pending {
                Some(Pending::BracketFwd) => {
                    this.pending = None;
                    this.hunk_step(true, cx);
                }
                Some(Pending::BracketBack) => {
                    this.pending = None;
                    this.hunk_step(false, cx);
                }
                _ => this.clear_pending("sequence cancelled"),
            }))
            // LEADER-menu actions
            .on_action(cx.listener(|this, _: &AcceptHunk, _, cx| this.accept(cx)))
            .on_action(cx.listener(|this, _: &RejectHunk, _, cx| this.reject(cx)))
            .on_action(cx.listener(|this, _: &RestoreHunk, _, cx| this.restore(cx)))
            .on_action(cx.listener(|this, _: &CommentOnLine, window, cx| {
                this.start_comment(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SendFeedback, _, cx| this.send_feedback(cx)))
            .on_action(cx.listener(|this, _: &EditorJump, _, cx| {
                this.jump_to_editor(true, cx)
            }))
            .on_action(cx.listener(|this, _: &WriteBackPreview, _, cx| {
                this.write_back(false, cx)
            }))
            .on_action(cx.listener(|this, _: &WriteBackAccepted, _, cx| {
                this.write_back(true, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleThread, _, cx| this.toggle_thread(cx)))
            .on_action(cx.listener(|this, _: &RestartSession, _, cx| this.restart_session(cx)))
            // Header
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0x2a2a3a))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .items_baseline()
                            .child(div().text_xl().child("lazydelta"))
                            .child(
                                div()
                                    .px_1()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(if insert { blue() } else { grey() })
                                    .text_size(px(10.))
                                    .text_color(if insert { blue() } else { grey() })
                                    .child(mode.clone()),
                            )
                            .when_some(
                                self.pending.map(|p| p.label().to_string()),
                                |el, label| {
                                    el.child(
                                        div()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(rgba(0xe0af6826))
                                            .text_size(px(10.))
                                            .text_color(yellow())
                                            .child(label),
                                    )
                                },
                            )
                            .when_some(self.agent_name.clone(), |el, agent| {
                                el.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(green())
                                        .child(format!("⚡ {agent}")),
                                )
                            })
                            .when(self.queue_sent > 0, |el| {
                                el.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(yellow())
                                        .child(format!("queue: {}", self.queue_sent)),
                                )
                            })
                            .child(
                                div()
                                    .text_color(grey())
                                    .text_size(px(11.))
                                    .child(if self.project.is_some() {
                                        "project".to_string()
                                    } else {
                                        "demo".to_string()
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .text_size(px(11.))
                            .child(div().text_color(grey()).child(
                                self.changeset
                                    .files
                                    .iter()
                                    .map(|f| f.path.display().to_string())
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            ))
                            .child(div().text_color(grey()).child(format!("· {} hunks", stats.0)))
                            .child(
                                div()
                                    .text_color(grey())
                                    .child(format!("· {} proposed", stats.1)),
                            )
                            .child(
                                div()
                                    .text_color(green())
                                    .child(format!("{} accepted", stats.2)),
                            )
                            .child(
                                div()
                                    .text_color(red())
                                    .child(format!("{} rejected", stats.3)),
                            ),
                    ),
            )
            // Diff body
            .child(
                div().id("diff-body").flex_1().overflow_y_scroll().p_3().child(
                    div().flex().flex_col().children(
                        self.changeset
                            .files
                            .iter()
                            .flat_map(|f| f.hunks.iter())
                            .map(|h| self.hunk_card(h, cx))
                            .collect::<Vec<_>>(),
                    ),
                ),
            )
            // Thread panel (collapsible, task 4.7)
            .when(!self.collapsed && !self.messages.is_empty(), |el| {
                el.child(
                    div()
                        .id("thread-panel")
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .border_t_1()
                        .border_color(rgb(0x2a2a3a))
                        .px_3()
                        .py_2()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(self.messages.iter().rev().map(|m| {
                            let is_agent = m.role == "agent";
                            div().flex().flex_col().child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(if is_agent { green() } else { blue() })
                                    .child(m.role.clone()),
                            )
                            .when(is_agent, |el| el.child(render_markdown(&m.text)))
                            .when(!is_agent, |el| {
                                el.child(
                                    div()
                                        .id("you-msg")
                                        .max_h(px(80.))
                                        .overflow_y_scroll()
                                        .text_size(px(11.))
                                        .text_color(grey())
                                        .child(m.text.clone()),
                                )
                            })
                        })),
                )
            })
            .when(self.collapsed && !self.messages.is_empty(), |el| {
                el.child(
                    div()
                        .border_t_1()
                        .border_color(rgb(0x2a2a3a))
                        .px_3()
                        .py_1()
                        .text_size(px(11.))
                        .text_color(grey())
                        .child(format!(
                            "thread: {} messages · {} queued · [space t] expand",
                            self.messages.len(),
                            self.queue_sent
                        )),
                )
            })
            // Feedback preview (what was sent, task 4.2)
            .when_some(self.feedback_preview.clone(), |el, preview| {
                el.child(
                    div()
                        .id("fb-preview")
                        .max_h(px(140.))
                        .overflow_y_scroll()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(0x33467c))
                        .bg(rgb(0x1a1b2a))
                        .p_2()
                        .text_size(px(11.))
                        .text_color(rgb(0xa9b1d6))
                        .child(preview),
                )
            })
            // Error banner (task 4.1)
            .when_some(self.error_banner.clone(), |el, error| {
                el.child(
                    div()
                        .id("error-banner")
                        .px_3()
                        .py_1()
                        .bg(rgba(0xf7768e26))
                        .border_t_1()
                        .border_color(red())
                        .text_size(px(11.))
                        .text_color(red())
                        .child(format!("{error} · [esc] dismiss")),
                )
            })
            // Footer
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .border_t_1()
                    .border_color(rgb(0x2a2a3a))
                    .px_3()
                    .py_2()
                    .when_some(self.comment_draft.clone(), |el, draft| {
                        let (hunk_id, line) = self.comment_target.unwrap();
                        let line_text = self
                            .changeset
                            .hunk(hunk_id)
                            .ok()
                            .and_then(|h| h.lines.get(line))
                            .map(|l| l.text.clone())
                            .unwrap_or_default();
                        el.child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .flex_none()
                                        .text_color(blue())
                                        .text_size(px(11.))
                                        .child("comment →"),
                                )
                                .child(
                                    div()
                                        .id("draft-line")
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(grey())
                                        .text_size(px(11.))
                                        .overflow_x_scroll()
                                        .child(line_text),
                                )
                                .child(draft),
                        )
                    })
                    .when_some(self.hint.clone(), |el, hint| {
                        el.child(
                            div()
                                .text_size(px(11.))
                                .text_color(yellow())
                                .child(hint),
                        )
                    })
                    .when(self.confirm.is_some(), |el| {
                        el.child(
                            div()
                                .text_size(px(11.))
                                .text_color(yellow())
                                .child("confirm: [y] write & continue · [n] cancel"),
                        )
                    })
                    .child(
                        div()
                            .id("footer-legend")
                            .flex()
                            .flex_row()
                            .gap_2()
                            .text_size(px(11.))
                            .text_color(grey())
                            .child("j/k gg G ]h [h nav")
                            .child("· space a/r/u accept·reject·restore")
                            .child("· i insert")
                            .child("· space c comment")
                            .child("· space s send")
                            .child("· space e / J editor")
                            .child("· space w/W write-back")
                            .child("· R refresh")
                            .child("· space t thread")
                            .child("· space x restart")
                            .child("· tab agent"),
                    ),
            )
    }
}

pub fn bind_review_keys(cx: &mut App) {
    use gpui::KeyBinding;
    let n = Some("ReviewNormal");
    let p = Some("ReviewPending");
    let l = Some("ReviewLeader");
    cx.bind_keys([
        // NORMAL navigation + mode keys
        KeyBinding::new("j", NavDown, n),
        KeyBinding::new("down", NavDown, n),
        KeyBinding::new("k", NavUp, n),
        KeyBinding::new("up", NavUp, n),
        KeyBinding::new("G", LastLine, n),
        KeyBinding::new("g", PendingG, n),
        KeyBinding::new("]", PendingBracketFwd, n),
        KeyBinding::new("[", PendingBracketBack, n),
        KeyBinding::new("space", LeaderKey, n),
        KeyBinding::new("i", InsertStart, n),
        KeyBinding::new("a", InsertStart, n),
        KeyBinding::new("J", JumpToEditor, n),
        KeyBinding::new("R", RefreshChangeset, n),
        KeyBinding::new("y", ConfirmYes, n),
        KeyBinding::new("n", ConfirmNo, n),
        KeyBinding::new("tab", CycleAgent, n),
        KeyBinding::new("escape", DismissNotice, n),
        // PENDING continuations
        KeyBinding::new("g", FirstLine, p),
        KeyBinding::new("h", HKey, p),
        KeyBinding::new("escape", DismissNotice, p),
        // LEADER menu (space-prefixed actions)
        KeyBinding::new("a", AcceptHunk, l),
        KeyBinding::new("r", RejectHunk, l),
        KeyBinding::new("u", RestoreHunk, l),
        KeyBinding::new("c", CommentOnLine, l),
        KeyBinding::new("s", SendFeedback, l),
        KeyBinding::new("e", EditorJump, l),
        KeyBinding::new("w", WriteBackPreview, l),
        KeyBinding::new("W", WriteBackAccepted, l),
        KeyBinding::new("t", ToggleThread, l),
        KeyBinding::new("x", RestartSession, l),
        KeyBinding::new("escape", DismissNotice, l),
    ]);
}
