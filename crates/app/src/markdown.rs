//! Markdown-subset rendering for agent messages (design D6).
//!
//! `parse_markdown` is pure (unit-testable); `render_markdown` maps the
//! parsed blocks onto GPUI elements. v1 subset: headings, paragraphs with
//! bold/italic/inline-code spans, fenced code blocks, lists, links as text.

use gpui::{FontWeight, div, prelude::*, px, rgb, rgba};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq)]
pub struct MdInline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MdBlock {
    Heading(u8, Vec<MdInline>),
    Paragraph(Vec<MdInline>),
    Code(String),
    List(Vec<Vec<MdInline>>),
}

#[derive(Debug, Clone, Default)]
struct Style {
    bold: bool,
    italic: bool,
}

/// Parse markdown into the v1 block model.
pub fn parse_markdown(src: &str) -> Vec<MdBlock> {
    let mut blocks: Vec<MdBlock> = Vec::new();
    let mut inlines: Vec<MdInline> = Vec::new();
    let mut style = Style::default();
    // Heading being collected (level, inlines).
    let mut heading: Option<(u8, Vec<MdInline>)> = None;
    // Fenced code block being collected.
    let mut code_buffer: Option<String> = None;
    // List items being collected.
    let mut list_items: Option<Vec<Vec<MdInline>>> = None;

    fn flush_inlines(
        inlines: &mut Vec<MdInline>,
        blocks: &mut Vec<MdBlock>,
        list_items: &mut Option<Vec<Vec<MdInline>>>,
    ) {
        if let Some(items) = list_items {
            if !inlines.is_empty() {
                items.push(std::mem::take(inlines));
            }
        } else if !inlines.is_empty() {
            blocks.push(MdBlock::Paragraph(std::mem::take(inlines)));
        }
    }

    for event in Parser::new_ext(src, Options::empty()) {
        // Inside a fenced code block everything is raw text.
        if let Some(code) = &mut code_buffer {
            match event {
                Event::Text(text) => code.push_str(&text),
                Event::End(TagEnd::CodeBlock) => {
                    let code = code_buffer.take().expect("code buffer");
                    blocks.push(MdBlock::Code(code));
                }
                _ => {}
            }
            continue;
        }

        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                flush_inlines(&mut inlines, &mut blocks, &mut list_items);
                code_buffer = Some(String::new());
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush_inlines(&mut inlines, &mut blocks, &mut list_items);
                style = Style::default();
                heading = Some((level as u8, Vec::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, inl)) = heading.take() {
                    blocks.push(MdBlock::Heading(level, inl));
                }
            }
            Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) => {
                if heading.is_none() {
                    flush_inlines(&mut inlines, &mut blocks, &mut list_items);
                }
            }
            Event::Start(Tag::List(_)) => {
                flush_inlines(&mut inlines, &mut blocks, &mut list_items);
                list_items = Some(Vec::new());
            }
            Event::End(TagEnd::List(_)) => {
                flush_inlines(&mut inlines, &mut blocks, &mut list_items);
                if let Some(items) = list_items.take().filter(|i| !i.is_empty()) {
                    blocks.push(MdBlock::List(items));
                }
            }
            Event::End(TagEnd::Item) => {
                if list_items.is_some() {
                    flush_inlines(&mut inlines, &mut blocks, &mut list_items);
                }
            }
            Event::Start(Tag::Strong) => style.bold = true,
            Event::End(TagEnd::Strong) => style.bold = false,
            Event::Start(Tag::Emphasis) => style.italic = true,
            Event::End(TagEnd::Emphasis) => style.italic = false,
            Event::Code(text) => push_inline(
                &mut inlines,
                &mut heading,
                MdInline {
                    text: text.to_string(),
                    bold: false,
                    italic: false,
                    code: true,
                },
            ),
            Event::Text(text) => push_inline(
                &mut inlines,
                &mut heading,
                MdInline {
                    text: text.to_string(),
                    bold: style.bold,
                    italic: style.italic,
                    code: false,
                },
            ),
            // Soft/hard breaks become spaces (flex-wrap handles line flow).
            Event::SoftBreak | Event::HardBreak => push_inline(
                &mut inlines,
                &mut heading,
                MdInline {
                    text: " ".into(),
                    bold: false,
                    italic: false,
                    code: false,
                },
            ),
            // Links render as their label text (design D6); code spans are
            // handled via Event::Code above.
            Event::Start(Tag::Link { .. }) | Event::End(TagEnd::Link) => {}
            _ => {}
        }
    }
    flush_inlines(&mut inlines, &mut blocks, &mut list_items);
    if let Some(items) = list_items.take().filter(|i| !i.is_empty()) {
        blocks.push(MdBlock::List(items));
    }
    blocks
}

fn push_inline(
    inlines: &mut Vec<MdInline>,
    heading: &mut Option<(u8, Vec<MdInline>)>,
    inline: MdInline,
) {
    match heading {
        Some((_, inl)) => inl.push(inline),
        None => inlines.push(inline),
    }
}

// ---- Rendering (GPUI) ----

fn text_col() -> gpui::Rgba {
    rgb(0xc0caf5)
}

pub fn render_markdown(src: &str) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .text_size(px(12.))
        .text_color(text_col())
        .children(parse_markdown(src).iter().map(render_block))
}

fn render_block(block: &MdBlock) -> impl IntoElement + use<> {
    match block {
        MdBlock::Heading(level, inl) => div()
            .text_size(match level {
                1 => px(16.),
                2 => px(15.),
                _ => px(14.),
            })
            .font_weight(FontWeight::BOLD)
            .child(render_inlines(inl)),
        MdBlock::Paragraph(inl) => div().child(render_inlines(inl)),
        MdBlock::Code(code) => div()
            .bg(rgb(0x1a1b2a))
            .border_1()
            .border_color(rgb(0x2a2a3a))
            .rounded_md()
            .px_2()
            .py_1()
            .flex()
            .flex_col()
            .children(code.lines().map(|line| div().child(line.to_string()))),
        MdBlock::List(items) => div()
            .flex()
            .flex_col()
            .gap_0()
            .children(items.iter().map(|item| {
                div()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .child(div().text_color(rgb(0x7aa2f7)).child("•"))
                    .child(render_inlines(item))
            })),
    }
}

fn render_inlines(inl: &[MdInline]) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_wrap()
        .gap_0()
        .children(inl.iter().map(|i| {
            let mut el = div().child(i.text.clone());
            if i.code {
                el = el
                    .bg(rgba(0x7aa2f72a))
                    .rounded_sm()
                    .px_1()
                    .text_color(rgb(0x9ece6a));
            }
            if i.bold {
                el = el.font_weight(FontWeight::BOLD);
            }
            if i.italic {
                el = el.italic();
            }
            el
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> MdInline {
        MdInline { text: text.into(), bold: false, italic: false, code: false }
    }

    #[test]
    fn parses_headings_paragraphs_and_spans() {
        let blocks = parse_markdown("# Title\n\nhello **bold** *it* `code`\n");
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            MdBlock::Heading(1, inl) => {
                assert!(inl.iter().any(|i| i.text == "Title"));
            }
            other => panic!("expected heading, got {other:?}"),
        }
        match &blocks[1] {
            MdBlock::Paragraph(inl) => {
                assert!(inl.iter().any(|i| i.bold && i.text == "bold"));
                assert!(inl.iter().any(|i| i.italic && i.text == "it"));
                assert!(inl.iter().any(|i| i.code && i.text == "code"));
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn parses_fenced_code_blocks() {
        let blocks = parse_markdown("before\n```rust\nfn x() {}\n```\nafter");
        assert_eq!(blocks.len(), 3);
        match &blocks[1] {
            MdBlock::Code(code) => assert_eq!(code, "fn x() {}\n"),
            other => panic!("expected code block, got {other:?}"),
        }
    }

    #[test]
    fn parses_lists_and_links_as_text() {
        let blocks = parse_markdown("- one **two**\n- [docs](https://x)\n");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            MdBlock::List(items) => {
                assert_eq!(items.len(), 2);
                assert!(items[0].iter().any(|i| i.bold && i.text == "two"));
                assert!(items[1].iter().any(|i| i.text == "docs"));
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn plain_text_is_a_paragraph() {
        let blocks = parse_markdown("plain");
        assert_eq!(blocks, vec![MdBlock::Paragraph(vec![plain("plain")])]);
    }
}
