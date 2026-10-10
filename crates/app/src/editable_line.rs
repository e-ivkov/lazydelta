//! A single-line editable text primitive (simplified from gpui's `input`
//! example): enough input handling to type, move, and commit a line.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement,
    KeyBinding, LayoutId, PaintQuad, Pixels, Point, Render, ShapedLine, Style, TextRun,
    UTF16Selection, Window, actions, div, fill, point, px, rgb, size,
};

actions!(
    editable_line,
    [LineLeft, LineRight, LineHome, LineEnd, LineBackspace, LineDelete, LineCommit, LineCancel]
);

pub enum EditEvent {
    Commit(String),
    Cancel,
}

pub struct EditableLine {
    pub focus_handle: FocusHandle,
    return_focus: FocusHandle,
    content: String,
    /// UTF-8 byte offset of the cursor (selection is not supported).
    cursor: usize,
    /// Vim semantics for line editing (task 6.3): `Esc` commits the edit
    /// instead of cancelling it (cancelling is for comment drafts).
    escape_commits: bool,
    #[allow(dead_code)]
    last_bounds: Option<Bounds<Pixels>>,
}

impl EventEmitter<EditEvent> for EditableLine {}

impl EditableLine {
    pub fn new(
        text: &str,
        return_focus: FocusHandle,
        escape_commits: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        EditableLine {
            focus_handle: cx.focus_handle(),
            return_focus,
            content: text.to_string(),
            cursor: text.len(),
            escape_commits,
            last_bounds: None,
        }
    }

    fn insert_at_cursor(&mut self, text: &str, cx: &mut Context<Self>) {
        let at = self.cursor;
        self.content.replace_range(at..at, text);
        self.cursor += text.len();
        cx.notify();
    }

    fn backspace(&mut self, cx: &mut Context<Self>) {
        if self.cursor > 0 {
            let prev = self.prev_boundary(self.cursor);
            self.content.replace_range(prev..self.cursor, "");
            self.cursor = prev;
            cx.notify();
        }
    }

    fn delete_forward(&mut self, cx: &mut Context<Self>) {
        if self.cursor < self.content.len() {
            let next = self.next_boundary(self.cursor);
            self.content.replace_range(self.cursor..next, "");
            cx.notify();
        }
    }

    fn prev_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .rev()
            .find_map(|(i, _)| (i < offset).then_some(i))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .find_map(|(i, _)| (i > offset).then_some(i))
            .unwrap_or(self.content.len())
    }

    fn commit(&mut self, _: &LineCommit, window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(EditEvent::Commit(self.content.clone()));
        let back = self.return_focus.clone();
        window.focus(&back);
    }

    fn cancel(&mut self, _: &LineCancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.escape_commits {
            // Vim insert mode: leaving with Esc keeps the changes (task 6.3).
            self.commit(&LineCommit, window, cx);
            return;
        }
        cx.emit(EditEvent::Cancel);
        let back = self.return_focus.clone();
        window.focus(&back);
    }
}

impl EntityInputHandler for EditableLine {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let start = self.offset_from_utf16(range_utf16.start);
        let end = self.offset_from_utf16(range_utf16.end);
        self.content.get(start..end).map(str::to_owned)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let cursor = self.offset_to_utf16(self.cursor);
        Some(UTF16Selection {
            range: cursor..cursor,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert_at_cursor(new_text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        _new_text: &str,
        _new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl EditableLine {
    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }
}

struct LinePrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
}

struct LineTextElement {
    input: Entity<EditableLine>,
}

impl IntoElement for LineTextElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for LineTextElement {
    type RequestLayoutState = ();
    type PrepaintState = LinePrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content: gpui::SharedString = input.content.clone().into();
        let cursor = input.cursor;
        let style = window.text_style();

        let run = TextRun {
            len: content.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(content, font_size, &[run], None);

        let cursor_pos = line.x_for_index(cursor);
        let cursor = fill(
            Bounds::new(
                point(bounds.left() + cursor_pos, bounds.top()),
                size(px(2.), bounds.bottom() - bounds.top()),
            ),
            rgb(0x7aa2f7),
        );
        LinePrepaintState {
            line: Some(line),
            cursor: Some(cursor),
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        let line = prepaint.line.take().unwrap();
        line.paint(bounds.origin, window.line_height(), window, cx)
            .unwrap();

        if focus_handle.is_focused(window) && let Some(cursor) = prepaint.cursor.take() {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _| {
            input.last_bounds = Some(bounds);
        });
    }
}

impl Focusable for EditableLine {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditableLine {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("EditableLine")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_row()
            .pl_1()
            .min_w(px(320.))
            .w_full()
            .rounded_sm()
            .bg(rgb(0x24283b))
            .border_1()
            .border_color(rgb(0x7aa2f7))
            .on_action(cx.listener(|this, _: &LineLeft, _, cx| {
                this.cursor = this.prev_boundary(this.cursor);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &LineRight, _, cx| {
                this.cursor = this.next_boundary(this.cursor);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &LineHome, _, cx| {
                this.cursor = 0;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &LineEnd, _, cx| {
                this.cursor = this.content.len();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &LineBackspace, _, cx| this.backspace(cx)))
            .on_action(cx.listener(|this, _: &LineDelete, _, cx| this.delete_forward(cx)))
            .on_action(cx.listener(Self::commit))
            .on_action(cx.listener(Self::cancel))
            .child(LineTextElement {
                input: cx.entity(),
            })
    }
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", LineLeft, Some("EditableLine")),
        KeyBinding::new("right", LineRight, Some("EditableLine")),
        KeyBinding::new("home", LineHome, Some("EditableLine")),
        KeyBinding::new("end", LineEnd, Some("EditableLine")),
        KeyBinding::new("backspace", LineBackspace, Some("EditableLine")),
        KeyBinding::new("delete", LineDelete, Some("EditableLine")),
        KeyBinding::new("enter", LineCommit, Some("EditableLine")),
        KeyBinding::new("escape", LineCancel, Some("EditableLine")),
    ]);
}
