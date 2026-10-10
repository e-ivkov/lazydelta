//! The review surface: renders a [`ChangeSet`] as hunks with provenance,
//! per-hunk accept/reject, editable added lines, and anchored comments.

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, MouseButton, SharedString, Window,
    actions, div, prelude::*, px, rgb, rgba,
};
use lazydelta_core::{ChangeSet, HunkId, HunkStatus, LineKind, MaterializePolicy, Provenance};

use crate::editable_line::{EditEvent, EditableLine};

actions!(
    review,
    [
        AcceptHunk,
        RejectHunk,
        RestoreHunk,
        EditFocusedLine,
        CommentOnLine,
        SendFeedback,
        FocusNextLine,
        FocusPrevLine
    ]
);

fn bg() -> gpui::Rgba { rgb(0x16161e) }
fn card() -> gpui::Rgba { rgb(0x1d1d28) }
fn card_rejected() -> gpui::Rgba { rgb(0x1a171c) }
fn added_bg() -> gpui::Rgba { rgba(0x9ece6a20) }
fn removed_bg() -> gpui::Rgba { rgba(0xf7768e1a) }
fn focus_bg() -> gpui::Rgba { rgba(0x7aa2f726) }
fn green() -> gpui::Rgba { rgb(0x9ece6a) }
fn red() -> gpui::Rgba { rgb(0xf7768e) }
fn blue() -> gpui::Rgba { rgb(0x7aa2f7) }
fn yellow() -> gpui::Rgba { rgb(0xe0af68) }
fn grey() -> gpui::Rgba { rgb(0x565f89) }
fn text_col() -> gpui::Rgba { rgb(0xc0caf5) }

pub struct ReviewView {
    pub focus_handle: FocusHandle,
    pub changeset: ChangeSet,
    focused: Option<(HunkId, usize)>,
    editing: Option<(HunkId, usize, Entity<EditableLine>)>,
    comment_target: Option<(HunkId, usize)>,
    comment_draft: Option<Entity<EditableLine>>,
    feedback_preview: Option<SharedString>,
}

impl ReviewView {
    pub fn new(changeset: ChangeSet, cx: &mut Context<Self>) -> Self {
        // Focus the first added line by default.
        let focused = changeset
            .files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .find_map(|h| {
                h.lines
                    .iter()
                    .position(|l| l.kind == LineKind::Added)
                    .map(|i| (h.id, i))
            });
        ReviewView {
            focus_handle: cx.focus_handle(),
            changeset,
            focused,
            editing: None,
            comment_target: None,
            comment_draft: None,
            feedback_preview: None,
        }
    }

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

    fn focus_next(&mut self, _: &FocusNextLine, _: &mut Window, cx: &mut Context<Self>) {
        let positions = self.added_line_positions();
        if positions.is_empty() {
            return;
        }
        let next = self
            .focused
            .and_then(|cur| positions.iter().position(|p| *p == cur))
            .map(|idx| (idx + 1) % positions.len())
            .unwrap_or(0);
        self.focused = Some(positions[next]);
        cx.notify();
    }

    fn focus_prev(&mut self, _: &FocusPrevLine, _: &mut Window, cx: &mut Context<Self>) {
        let positions = self.added_line_positions();
        if positions.is_empty() {
            return;
        }
        let prev = self
            .focused
            .and_then(|cur| positions.iter().position(|p| *p == cur))
            .map(|idx| idx.wrapping_sub(1).min(positions.len() - 1))
            .unwrap_or(0);
        self.focused = Some(positions[prev]);
        cx.notify();
    }

    fn focused_hunk(&mut self) -> Option<HunkId> {
        self.focused.map(|(h, _)| h)
    }

    fn accept(&mut self, _: &AcceptHunk, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(h) = self.focused_hunk() {
            let _ = self.changeset.accept_hunk(h);
            cx.notify();
        }
    }

    fn reject(&mut self, _: &RejectHunk, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(h) = self.focused_hunk() {
            let _ = self.changeset.reject_hunk(h);
            cx.notify();
        }
    }

    fn restore(&mut self, _: &RestoreHunk, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(h) = self.focused_hunk() {
            let _ = self.changeset.restore_hunk(h);
            cx.notify();
        }
    }

    fn start_edit(&mut self, _: &EditFocusedLine, window: &mut Window, cx: &mut Context<Self>) {
        let Some((hunk_id, line)) = self.focused else {
            return;
        };
        let Ok(hunk) = self.changeset.hunk(hunk_id) else {
            return;
        };
        if hunk.lines[line].kind != LineKind::Added {
            return;
        }
        let text = hunk.lines[line].text.clone();
        let return_focus = self.focus_handle.clone();
        let entity = cx.new(|cx| EditableLine::new(&text, return_focus, cx));
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
        }
        cx.notify();
    }

    fn start_comment(&mut self, _: &CommentOnLine, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.focused else {
            return;
        };
        let return_focus = self.focus_handle.clone();
        let entity = cx.new(|cx| EditableLine::new("", return_focus, cx));
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
        self.comment_target = Some(target);
        self.comment_draft = Some(entity.clone());
        window.focus(&handle);
        cx.notify();
    }

    fn finish_comment(&mut self, text: String, cx: &mut Context<Self>) {
        if let (Some((hunk_id, line)), Some(_)) = (self.comment_target.take(), self.comment_draft.take()) {
            if !text.trim().is_empty() {
                let _ = self.changeset.add_comment(hunk_id, Some(line), "you", text.trim());
            }
        }
        cx.notify();
    }

    fn send_feedback(&mut self, _: &SendFeedback, _: &mut Window, cx: &mut Context<Self>) {
        let md = self.changeset.feedback_markdown();
        println!("{md}");
        self.feedback_preview = Some(md.into());
        cx.notify();
    }

    #[allow(dead_code)] // wired up in the add-agent-review-loop change
    fn materialize_preview(&self) -> SharedString {
        let out = self.changeset.materialize(MaterializePolicy::Proposed);
        let mut text = String::new();
        for (path, contents) in out {
            text.push_str(&format!("── {} ──\n{}\n", path.display(), contents));
        }
        text.into()
    }

    fn provenance_badge(provenance: &Provenance) -> (&'static str, gpui::Rgba) {
        match provenance {
            Provenance::Agent { agent, turn } => {
                let _ = (agent, turn);
                ("AGENT", blue())
            }
            Provenance::Human => ("HUMAN", yellow()),
            Provenance::HumanFix => ("HUMAN FIX", yellow()),
        }
    }

    fn status_chip(status: HunkStatus) -> (&'static str, gpui::Rgba) {
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

        let (marker, marker_color, bg) = match line.kind {
            LineKind::Context => (" ", grey(), None),
            LineKind::Added => ("+", green(), Some(added_bg())),
            LineKind::Removed => ("-", red(), Some(removed_bg())),
        };

        let mut row = div().id(("line", hunk_id as u32 * 1000 + line_idx as u32)).flex().flex_row();        if is_focused {
            row = row.bg(focus_bg()).border_l_2().border_color(blue());
        } else {
            row = row.border_l_2().border_color(gpui::transparent_black());
            if let Some(bg) = bg {
                row = row.bg(bg);
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
        .when(is_focused && line.kind == LineKind::Added && !is_editing, |row| {
            row.child(
                div()
                    .flex_none()
                    .pr_2()
                    .text_size(px(11.))
                    .text_color(grey())
                    .child("e edit · c comment"),
            )
        })
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

    fn hunk_card(&self, hunk: &lazydelta_core::Hunk, cx: &Context<Self>) -> impl IntoElement + use<> {
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
            .children(hunk.lines.iter().enumerate().map(|(i, line)| {
                self.line_and_comments(hunk.id, i, line, cx)
            }))
    }

    /// A diff line followed by any comments anchored to it.
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
            .map(|h| h.comments.iter().filter(|c| c.line == Some(line_idx)).collect())
            .unwrap_or_default();

        let row = self.line_row(hunk_id, line_idx, line, cx);

        div()
            .flex()
            .flex_col()
            .child(row)
            .children(comments.into_iter().map(|c| {
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
            }))
    }
}

impl Focusable for ReviewView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ReviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.editing.is_some() || self.comment_draft.is_some();
        let key_context = if busy { "ReviewSurfaceBusy" } else { "ReviewSurface" };

        let stats = self.changeset.files.iter().fold((0, 0, 0, 0), |mut acc, f| {
            for h in &f.hunks {
                acc.0 += 1;
                match h.status {
                    HunkStatus::Proposed => acc.1 += 1,
                    HunkStatus::Accepted => acc.2 += 1,
                    HunkStatus::Rejected => acc.3 += 1,
                }
            }
            acc
        });

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
            .on_action(cx.listener(Self::accept))
            .on_action(cx.listener(Self::reject))
            .on_action(cx.listener(Self::restore))
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_prev))
            .on_action(cx.listener(Self::start_edit))
            .on_action(cx.listener(Self::start_comment))
            .on_action(cx.listener(Self::send_feedback))
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
                                    .text_color(grey())
                                    .text_size(px(11.))
                                    .child("provenance-aware review surface (demo)"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .text_size(px(11.))
                            .child(div().text_color(grey()).child(self.changeset.files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>().join(", ")))
                            .child(div().text_color(grey()).child(format!("· {} hunks", stats.0)))
                            .child(div().text_color(grey()).child(format!("· {} proposed", stats.1)))
                            .child(div().text_color(green()).child(format!("{} accepted", stats.2)))
                            .child(div().text_color(red()).child(format!("{} rejected", stats.3))),
                    ),
            )
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
                    .when_some(self.feedback_preview.clone(), |el, preview| {
                        el.child(
                            div()
                                .id("fb-preview")
                                .max_h(px(220.))
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
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .text_size(px(11.))
                            .text_color(grey())
                            .child("↑↓ focus added line")
                            .child("· a accept")
                            .child("· r reject")
                            .child("· u restore")
                            .child("· e edit line")
                            .child("· c comment")
                            .child("· s send feedback"),
                    ),
            )
    }
}
