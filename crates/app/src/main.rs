//! lazydelta: an agent-first review surface with an editable diff.
//!
//! Demo mode renders a hardcoded changeset so the review UX (provenance
//! badges, accept/reject, editable added lines, anchored comments, feedback
//! serialization) can be explored without an agent attached.

mod editable_line;
mod review;

use gpui::{
    App, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, actions,
    prelude::*, px, size,
};
use lazydelta_core::{ChangeSet, LineKind, Provenance};

use crate::review::ReviewView;

actions!(lazydelta, [Quit]);

const DEMO_BASE: &str = r#"use std::collections::HashMap;

pub struct Checkout {
    items: HashMap<String, u32>,
    total: u32,
}

impl Checkout {
    pub fn new() -> Self {
        Checkout {
            items: HashMap::new(),
            total: 0,
        }
    }

    pub fn add(&mut self, sku: &str, price: u32) {
        self.items.insert(sku.to_string(), price);
        self.total += price;
    }

    pub fn total(&self) -> u32 {
        self.total
    }
}
"#;

const DEMO_PROPOSED: &str = r#"use std::collections::HashMap;

#[derive(Debug)]
pub struct Checkout {
    items: HashMap<String, u32>,
    total: u32,
}

impl Checkout {
    pub fn new() -> Self {
        Checkout {
            items: HashMap::new(),
            total: 0,
        }
    }

    pub fn add(&mut self, sku: &str, price: u32) {
        self.items.insert(sku.to_string(), price);
        self.total += price;
        tracing::debug!(sku, price, "item added");
    }

    pub fn total(&self) -> u32 {
        self.total
    }
}
"#;

fn demo_changeset() -> ChangeSet {
    let mut cs = ChangeSet::from_diff(
        "src/checkout.rs",
        DEMO_BASE,
        DEMO_PROPOSED,
        Provenance::Agent {
            agent: "claude-code".into(),
            turn: 1,
        },
    );
    // Seed one anchored comment so threads are visible immediately.
    if let Some(file) = cs.files.first_mut() {
        if let Some(hunk) = file.hunks.iter_mut().find(|h| {
            h.lines
                .iter()
                .any(|l| l.text.contains("tracing::debug"))
        }) {
            if let Some(idx) = hunk
                .lines
                .iter()
                .position(|l| l.text.contains("tracing::debug"))
            {
                let _ = hunk.comments.push(lazydelta_core::Comment {
                    author: "you".into(),
                    body: "tracing on the hot path — guard behind a feature flag?".into(),
                    line: Some(idx),
                    replies: vec![lazydelta_core::Comment {
                        author: "claude-code".into(),
                        body: "Fair. I'll gate it behind `debug_assertions`.".into(),
                        line: Some(idx),
                        replies: Vec::new(),
                    }],
                });
                // keep clippy quiet about unused kind import in this fn
                let _ = LineKind::Added;
            }
        }
    }
    cs
}

fn load_fonts(cx: &mut App) {
    let font: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
    let _ = cx.text_system().add_fonts(vec![font.into()]);
}

fn main() {
    Application::new().run(|cx: &mut App| {
        load_fonts(cx);
        editable_line::bind_keys(cx);
        cx.bind_keys([
            KeyBinding::new("a", review::AcceptHunk, Some("ReviewSurface")),
            KeyBinding::new("r", review::RejectHunk, Some("ReviewSurface")),
            KeyBinding::new("u", review::RestoreHunk, Some("ReviewSurface")),
            KeyBinding::new("e", review::EditFocusedLine, Some("ReviewSurface")),
            KeyBinding::new("c", review::CommentOnLine, Some("ReviewSurface")),
            KeyBinding::new("s", review::SendFeedback, Some("ReviewSurface")),
            KeyBinding::new("down", review::FocusNextLine, Some("ReviewSurface")),
            KeyBinding::new("up", review::FocusPrevLine, Some("ReviewSurface")),
            KeyBinding::new("ctrl-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let changeset = demo_changeset();
        let bounds = Bounds::centered(None, size(px(1080.), px(760.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("lazydelta".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| ReviewView::new(changeset, cx)),
            )
            .expect("failed to open window");

        window
            .update(cx, |view, window, cx| {
                let handle = view.focus_handle.clone();
                window.focus(&handle);
                cx.activate(true);
            })
            .expect("failed to focus window");
    });
}
