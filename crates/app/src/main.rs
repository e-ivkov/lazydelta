//! lazydelta: an agent-first review surface with an editable diff.
//!
//! Modes:
//!   lazydelta                     # demo changeset, no agent required
//!   lazydelta --project <dir>     # review a real directory (git or snapshot)
//!   lazydelta --project <dir> --snapshot   # force snapshot base (fallback
//!                                          # for unreadable git metadata)
//!   lazydelta --thread <id>       # thread id under .lazydelta/threads (default: main)

mod editable_line;
mod review;

use gpui::{
    App, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, actions,
    prelude::*, px, size,
};
use lazydelta_app::config::Config;
use lazydelta_app::project::Project;
use lazydelta_app::store::{ThreadDocument, ThreadStore};
use lazydelta_core::{ChangeSet, Comment, Provenance};

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
    if let Some(file) = cs.files.first_mut()
        && let Some(hunk) = file
            .hunks
            .iter_mut()
            .find(|h| h.lines.iter().any(|l| l.text.contains("tracing::debug")))
        && let Some(idx) = hunk
            .lines
            .iter()
            .position(|l| l.text.contains("tracing::debug"))
    {
        hunk.comments.push(Comment {
            author: "you".into(),
            body: "tracing on the hot path — guard behind a feature flag?".into(),
            line: Some(idx),
            replies: vec![Comment {
                author: "claude-code".into(),
                body: "Fair. I'll gate it behind `debug_assertions`.".into(),
                line: Some(idx),
                replies: Vec::new(),
            }],
        });
    }
    cs
}

fn load_fonts(cx: &mut App) {
    let font: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
    let _ = cx.text_system().add_fonts(vec![font.into()]);
}

struct Args {
    demo: bool,
    project: Option<std::path::PathBuf>,
    force_snapshot: bool,
    thread: String,
}

fn parse_args() -> Args {
    let mut args = Args {
        demo: true,
        project: None,
        force_snapshot: false,
        thread: "main".into(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--demo" => args.demo = true,
            "--project" | "-p" => {
                if let Some(dir) = it.next() {
                    args.project = Some(dir.into());
                    args.demo = false;
                }
            }
            "--snapshot" => args.force_snapshot = true,
            "--thread" | "-t" => {
                if let Some(t) = it.next() {
                    args.thread = t;
                }
            }
            other => eprintln!("lazydelta: unknown flag {other}"),
        }
    }
    args
}

fn main() {
    let args = parse_args();
    Application::new().run(move |cx: &mut App| {
        load_fonts(cx);
        editable_line::bind_keys(cx);
        review::bind_review_keys(cx);
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        // Build the initial view state: demo changeset or a real project.
        let mut changeset = demo_changeset();
        let mut project_ctx: Option<(Project, Config, String, Option<ThreadDocument>)> = None;
        if let Some(root) = args.project.clone() {
            match Project::open(&root, &args.thread, args.force_snapshot) {
                Ok((project, cs, persisted)) => {
                    let config = Config::load(&root).unwrap_or_else(|e| {
                        eprintln!("lazydelta: config error: {e}");
                        Config::default()
                    });
                    changeset = cs;
                    project_ctx = Some((project, config, args.thread.clone(), persisted));
                }
                Err(e) => {
                    if lazydelta_core::git::is_git_repo(&root) && !args.force_snapshot {
                        eprintln!(
                            "lazydelta: git metadata looks unreadable ({e});\n\
                             retry with --snapshot to review against a captured base instead"
                        );
                        std::process::exit(1);
                    }
                    eprintln!("lazydelta: cannot open project: {e}");
                    std::process::exit(1);
                }
            }
        } else if args.force_snapshot {
            eprintln!("lazydelta: --snapshot requires --project <dir>");
            std::process::exit(1);
        }

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
                |_, cx| {
                    cx.new(|cx| {
                        let mut view = ReviewView::new(changeset, cx);
                        if let Some((project, config, thread, persisted)) = project_ctx {
                            view.attach_project(project, config, thread, persisted, cx);
                        }
                        view
                    })
                },
            )
            .expect("failed to open window");

        window
            .update(cx, |view, window, cx| {
                let handle = view.focus_handle.clone();
                window.focus(&handle);
                cx.activate(true);
            })
            .expect("failed to focus window");

        // Surface thread-load errors after the window is up.
        if let Some(root) = &args.project {
            let (_, errors) = ThreadStore::new(root).load_all();
            for err in errors {
                eprintln!("lazydelta: thread failed to load: {err}");
            }
        }
    });
}
