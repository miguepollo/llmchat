use eframe::egui;
use eframe::egui::text::LayoutJob;
use eframe::egui::{Color32, FontFamily, FontId, RichText, TextFormat, Ui};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// AST mínimo para renderizar markdown en egui
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Inline {
    Text(String),
    Code(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link(String, Vec<Inline>),
    Break,
}

#[derive(Debug)]
pub enum Block {
    Para(Vec<Inline>),
    Heading(u8, Vec<Inline>),
    Code { lang: String, text: String },
    List { ordered: bool, items: Vec<Vec<Block>> },
    Quote(Vec<Block>),
    Rule,
    Raw(String),
}

// ---------------------------------------------------------------------------
// Parseo (pulldown-cmark -> AST)
// ---------------------------------------------------------------------------

pub fn parse(text: &str) -> Vec<Block> {
    let mut queue: VecDeque<Event<'_>> = Parser::new_ext(text, Options::all()).collect();
    parse_blocks(&mut queue, &|_| false)
}

fn parse_blocks<'a>(
    queue: &mut VecDeque<Event<'a>>,
    is_end: &dyn Fn(&Event<'a>) -> bool,
) -> Vec<Block> {
    let mut out = Vec::new();
    while let Some(event) = queue.pop_front() {
        if is_end(&event) {
            break;
        }
        match event {
            Event::Start(Tag::Paragraph) => {
                let inlines = parse_inlines(queue);
                if !inlines.is_empty() {
                    out.push(Block::Para(inlines));
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let inlines = parse_inlines(queue);
                out.push(Block::Heading(level_number(level), inlines));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                let mut code = String::new();
                while let Some(e) = queue.pop_front() {
                    match e {
                        Event::End(TagEnd::CodeBlock) => break,
                        Event::Text(t) => code.push_str(&t),
                        _ => {}
                    }
                }
                while code.ends_with('\n') {
                    code.pop();
                }
                out.push(Block::Code { lang, text: code });
            }
            Event::Start(Tag::List(start)) => {
                let ordered = start.is_some();
                let mut items = Vec::new();
                loop {
                    match queue.pop_front() {
                        Some(Event::Start(Tag::Item)) => {
                            let item = parse_blocks(queue, &|e| matches!(e, Event::End(TagEnd::Item)));
                            items.push(item);
                        }
                        Some(Event::End(TagEnd::List(_))) => break,
                        Some(_) => continue,
                        None => break,
                    }
                }
                out.push(Block::List { ordered, items });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                let inner =
                    parse_blocks(queue, &|e| matches!(e, Event::End(TagEnd::BlockQuote(_))));
                out.push(Block::Quote(inner));
            }
            Event::Start(Tag::Table(_)) => {
                // Las tablas se aplanan a texto plano (suficiente para un chat).
                let mut buf = String::new();
                while let Some(e) = queue.pop_front() {
                    match e {
                        Event::End(TagEnd::Table) => break,
                        Event::Text(t) => buf.push_str(&t),
                        Event::SoftBreak | Event::HardBreak => buf.push('\n'),
                        _ => {}
                    }
                }
                out.push(Block::Para(vec![Inline::Text(buf)]));
            }
            Event::Rule => out.push(Block::Rule),
            Event::Text(t) => out.push(Block::Raw(t.to_string())),
            Event::End(_) => {}
            _ => {}
        }
    }
    out
}

fn parse_inlines<'a>(queue: &mut VecDeque<Event<'a>>) -> Vec<Inline> {
    parse_inlines_until(
        queue,
        &|e| matches!(e, Event::End(TagEnd::Paragraph) | Event::End(TagEnd::Heading(_))),
    )
}

fn parse_inlines_until<'a>(
    queue: &mut VecDeque<Event<'a>>,
    is_end: &dyn Fn(&Event<'a>) -> bool,
) -> Vec<Inline> {
    let mut out = Vec::new();
    while let Some(event) = queue.pop_front() {
        if is_end(&event) {
            break;
        }
        match event {
            Event::Text(t) => push_text(&mut out, t.to_string()),
            Event::Code(c) => out.push(Inline::Code(c.to_string())),
            Event::SoftBreak | Event::HardBreak => out.push(Inline::Break),
            Event::Start(Tag::Emphasis) => {
                let inner = parse_inlines_until(queue, &|e| matches!(e, Event::End(TagEnd::Emphasis)));
                out.push(Inline::Emphasis(inner));
            }
            Event::Start(Tag::Strong) => {
                let inner = parse_inlines_until(queue, &|e| matches!(e, Event::End(TagEnd::Strong)));
                out.push(Inline::Strong(inner));
            }
            Event::Start(Tag::Strikethrough) => {
                let inner =
                    parse_inlines_until(queue, &|e| matches!(e, Event::End(TagEnd::Strikethrough)));
                out.push(Inline::Strikethrough(inner));
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let inner = parse_inlines_until(queue, &|e| matches!(e, Event::End(TagEnd::Link)));
                out.push(Inline::Link(dest_url.to_string(), inner));
            }
            Event::Start(Tag::Image { dest_url, title, .. }) => {
                let _ = parse_inlines_until(queue, &|e| matches!(e, Event::End(TagEnd::Image)));
                let label = if title.is_empty() {
                    dest_url.to_string()
                } else {
                    title.to_string()
                };
                out.push(Inline::Text(format!("[imagen: {label}]")));
            }
            Event::End(TagEnd::Paragraph) | Event::End(TagEnd::Heading(_)) => break,
            _ => {}
        }
    }
    out
}

fn push_text(out: &mut Vec<Inline>, text: String) {
    let mut parts = text.split('\n').peekable();
    while let Some(part) = parts.next() {
        if !part.is_empty() {
            out.push(Inline::Text(part.to_string()));
        }
        if parts.peek().is_some() {
            out.push(Inline::Break);
        }
    }
}

fn level_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

// ---------------------------------------------------------------------------
// Renderizado en egui
// ---------------------------------------------------------------------------

pub fn render(ui: &mut Ui, text: &str) {
    let blocks = parse(text);
    for block in &blocks {
        render_block(ui, block);
    }
}

fn render_block(ui: &mut Ui, block: &Block) {
    match block {
        Block::Para(inlines) => {
            let mut job = LayoutJob::default();
            append_inlines(&mut job, inlines, body_format(ui, 13.5));
            ui.add(egui::Label::new(job).wrap());
        }
        Block::Heading(level, inlines) => {
            let size = match level {
                1 => 20.0,
                2 => 17.0,
                3 => 15.0,
                _ => 14.0,
            };
            let mut job = LayoutJob::default();
            let fmt = strong_format(ui, size);
            append_inlines(&mut job, inlines, fmt);
            ui.add(egui::Label::new(job).wrap());
            ui.add_space(3.0);
        }
        Block::Code { lang, text } => render_code_block(ui, lang, text),
        Block::List { ordered, items } => {
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}.", i + 1)
                } else {
                    "•".to_string()
                };
                ui.horizontal(|ui| {
                    ui.label(marker);
                    ui.add_space(6.0);
                    render_blocks(ui, item);
                });
            }
            ui.add_space(4.0);
        }
        Block::Quote(inner) => {
            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| render_blocks(ui, inner));
            ui.add_space(4.0);
        }
        Block::Rule => {
            ui.separator();
            ui.add_space(4.0);
        }
        Block::Raw(text) => {
            ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
        }
    }
    ui.add_space(4.0);
}

fn render_blocks(ui: &mut Ui, blocks: &[Block]) {
    for block in blocks {
        render_block(ui, block);
    }
}

fn render_code_block(ui: &mut Ui, lang: &str, text: &str) {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if !lang.is_empty() {
                    ui.weak(lang);
                    ui.add_space(8.0);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Copiar").clicked() {
                        ui.ctx().copy_text(text.to_string());
                    }
                });
            });
            let mut job = LayoutJob::default();
            job.append(
                text,
                0.0,
                TextFormat {
                    font_id: FontId::monospace(13.0),
                    color: ui.visuals().text_color(),
                    ..Default::default()
                },
            );
            ui.add(egui::Label::new(job).wrap());
        });
}

fn body_format(ui: &Ui, size: f32) -> TextFormat {
    TextFormat {
        font_id: FontId::proportional(size),
        color: ui.visuals().text_color(),
        ..Default::default()
    }
}

fn strong_format(ui: &Ui, size: f32) -> TextFormat {
    let mut format = body_format(ui, size);
    format.font_id.family = FontFamily::Name("Ubuntu-Medium".into());
    format
}

fn append_inlines(job: &mut LayoutJob, inlines: &[Inline], fmt: TextFormat) {
    for inline in inlines {
        match inline {
            Inline::Text(t) => job.append(t, 0.0, fmt.clone()),
            Inline::Code(c) => {
                let mut f = fmt.clone();
                f.font_id = FontId::monospace(fmt.font_id.size);
                f.background = Color32::from_gray(60);
                job.append(&format!(" {} ", c), 0.0, f);
            }
            Inline::Emphasis(inner) => {
                let mut f = fmt.clone();
                f.italics = true;
                append_inlines(job, inner, f);
            }
            Inline::Strong(inner) => {
                let mut f = fmt.clone();
                f.font_id.family = FontFamily::Name("Ubuntu-Medium".into());
                append_inlines(job, inner, f);
            }
            Inline::Strikethrough(inner) => {
                let mut f = fmt.clone();
                f.strikethrough = egui::Stroke::new(1.0_f32, fmt.color);
                append_inlines(job, inner, f);
            }
            Inline::Link(url, inner) => {
                let mut f = fmt.clone();
                f.color = Color32::from_rgb(0x62, 0x9c, 0xf0);
                f.underline = egui::Stroke::new(1.0_f32, f.color);
                append_inlines(job, inner, f);
                let _ = url; // enlaces visibles; el clic se puede añadir más adelante
            }
            Inline::Break => {
                job.append("\n", 0.0, fmt.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paragraph_with_inline_styles() {
        let blocks = parse("Hola **mundo** con `código` y *énfasis*.");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Para(inlines) => {
                assert!(matches!(inlines[0], Inline::Text(_)));
                assert!(matches!(inlines[1], Inline::Strong(_)));
                assert!(matches!(inlines[2], Inline::Text(_)));
                assert!(matches!(inlines[3], Inline::Code(_)));
                assert!(matches!(inlines[4], Inline::Text(_)));
                assert!(matches!(inlines[5], Inline::Emphasis(_)));
            }
            other => panic!("esperaba párrafo, got {other:?}"),
        }
    }

    #[test]
    fn parses_fenced_code_block() {
        let blocks = parse("```rust\nfn main() {}\n```");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Code { lang, text } => {
                assert_eq!(lang, "rust");
                assert_eq!(text, "fn main() {}");
            }
            other => panic!("esperaba bloque de código, got {other:?}"),
        }
    }

    #[test]
    fn parses_heading_and_list() {
        let blocks = parse("# Título\n\n- uno\n- dos");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], Block::Heading(1, _)));
        match &blocks[1] {
            Block::List { ordered, items } => {
                assert!(!ordered);
                assert_eq!(items.len(), 2);
            }
            other => panic!("esperaba lista, got {other:?}"),
        }
    }
}

