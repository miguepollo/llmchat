use eframe::egui;
use eframe::egui::text::LayoutJob;
use eframe::egui::{Color32, FontFamily, FontId, RichText, TextFormat, Ui};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// AST mínimo para renderizar markdown en egui
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Inline {
    Text(String),
    Code(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link(String, Vec<Inline>),
    Break,
}

#[derive(Debug, Clone)]
pub enum Block {
    Para(Vec<Inline>),
    Heading(u8, Vec<Inline>),
    Code { lang: String, text: String },
    List { ordered: bool, items: Vec<Vec<Block>> },
    Quote(Vec<Block>),
    Table { headers: Vec<Vec<Inline>>, rows: Vec<Vec<Vec<Inline>>> },
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
                out.push(parse_table(queue));
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

fn parse_table<'a>(queue: &mut VecDeque<Event<'a>>) -> Block {
    let mut headers: Vec<Vec<Inline>> = Vec::new();
    let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();
    while let Some(e) = queue.pop_front() {
        match e {
            Event::End(TagEnd::Table) => break,
            Event::Start(Tag::TableHead) => {
                while let Some(e2) = queue.pop_front() {
                    match e2 {
                        Event::End(TagEnd::TableHead) => break,
                        Event::Start(Tag::TableCell) => {
                            headers.push(parse_table_cell(queue));
                        }
                        _ => {}
                    }
                }
            }
            Event::Start(Tag::TableRow) => {
                let mut row: Vec<Vec<Inline>> = Vec::new();
                while let Some(e2) = queue.pop_front() {
                    match e2 {
                        Event::End(TagEnd::TableRow) => break,
                        Event::Start(Tag::TableCell) => {
                            row.push(parse_table_cell(queue));
                        }
                        _ => {}
                    }
                }
                rows.push(row);
            }
            _ => {}
        }
    }
    Block::Table { headers, rows }
}

/// El contenido de una celda puede venir envuelto en un párrafo o directo.
fn parse_table_cell<'a>(queue: &mut VecDeque<Event<'a>>) -> Vec<Inline> {
    if matches!(queue.front(), Some(Event::Start(Tag::Paragraph))) {
        queue.pop_front();
        let inlines =
            parse_inlines_until(queue, &|e| matches!(e, Event::End(TagEnd::Paragraph)));
        queue.pop_front(); // consume el End(Paragraph)
        inlines
    } else {
        parse_inlines_until(queue, &|e| matches!(e, Event::End(TagEnd::TableCell)))
    }
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

pub fn render(ui: &mut Ui, text: &str, scale: f32) {
    let blocks = parse(text);
    for block in &blocks {
        render_block(ui, block, scale);
    }
}

fn render_block(ui: &mut Ui, block: &Block, scale: f32) {
    match block {
        Block::Para(inlines) => {
            render_paragraph(ui, inlines, scale);
        }
        Block::Heading(level, inlines) => {
            let size = match level {
                1 => 20.0,
                2 => 17.0,
                3 => 15.0,
                _ => 14.0,
            };
            let mut job = LayoutJob::default();
            let fmt = strong_format(ui, scaled_size(size, scale));
            append_inlines(&mut job, inlines, fmt);
            ui.add(egui::Label::new(job).wrap());
            ui.add_space(3.0);
        }
        Block::Code { lang, text } => render_code_block(ui, lang, text, scale),
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
                    render_blocks(ui, item, scale);
                });
            }
            ui.add_space(4.0);
        }
        Block::Quote(inner) => {
            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| render_blocks(ui, inner, scale));
            ui.add_space(4.0);
        }
        Block::Rule => {
            ui.separator();
            ui.add_space(4.0);
        }
        Block::Table { headers, rows } => {
            render_table(ui, headers, rows);
        }
        Block::Raw(text) => {
            ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
        }
    }
    ui.add_space(4.0);
}

fn render_blocks(ui: &mut Ui, blocks: &[Block], scale: f32) {
    for block in blocks {
        render_block(ui, block, scale);
    }
}

/// Renderiza un párrafo. Si no contiene enlaces usa la ruta eficiente con
/// un único LayoutJob (buen envoltorio de línea); si contiene enlaces,
/// dibuja esos enlaces como widgets clicables manteniendo el texto fluido.
fn render_paragraph(ui: &mut Ui, inlines: &[Inline], scale: f32) {
    let body = body_format(ui, scaled_size(13.5, scale));
    let has_link = inlines.iter().any(|i| matches!(i, Inline::Link(_, _)));
    if !has_link {
        let mut job = LayoutJob::default();
        append_inlines(&mut job, inlines, body);
        ui.add(egui::Label::new(job).wrap());
        return;
    }
    ui.horizontal_wrapped(|ui| {
        let mut pending = LayoutJob::default();
        let mut dirty = false;
        for inline in inlines {
            if let Inline::Link(url, inner) = inline {
                if dirty {
                    ui.add(egui::Label::new(std::mem::take(&mut pending)).wrap());
                    dirty = false;
                }
                ui.add(egui::Hyperlink::from_label_and_url(inline_plain(inner), url.clone()));
            } else {
                append_inline(&mut pending, inline, body.clone());
                dirty = true;
            }
        }
        if dirty {
            ui.add(egui::Label::new(pending).wrap());
        }
    });
}

/// Dibuja una tabla como una retícula con encabezado resaltado y filas alternas.
fn render_table(ui: &mut Ui, headers: &[Vec<Inline>], rows: &[Vec<Vec<Inline>>]) {
    let cols = headers
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
    if cols == 0 {
        return;
    }
    let mut header = headers.to_vec();
    // Rellena celdas vacías para que egui::Grid mantenga columnas alineadas.
    while header.len() < cols {
        header.push(Vec::new());
    }
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            egui::Grid::new("md_table")
                .striped(true)
                .spacing([18.0, 4.0])
                .show(ui, |ui| {
                    for cell in &header {
                        ui.label(egui::RichText::new(inline_plain(cell)).strong());
                    }
                    ui.end_row();
                    for row in rows {
                        for cell in row {
                            ui.label(inline_plain(cell));
                        }
                        ui.end_row();
                    }
                });
        });
    ui.add_space(4.0);
}

/// Convierte inlines a texto plano (sin formato) para etiquetas y tablas.
fn inline_plain(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            Inline::Text(t) => s.push_str(t),
            Inline::Code(c) => {
                s.push('`');
                s.push_str(c);
                s.push('`');
            }
            Inline::Emphasis(inner)
            | Inline::Strong(inner)
            | Inline::Strikethrough(inner)
            | Inline::Link(_, inner) => {
                s.push_str(&inline_plain(inner));
            }
            Inline::Break => s.push(' '),
        }
    }
    s
}

fn render_code_block(ui: &mut Ui, lang: &str, text: &str, scale: f32) {
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
                    font_id: FontId::monospace(scaled_size(13.0, scale)),
                    color: ui.visuals().text_color(),
                    ..Default::default()
                },
            );
            ui.add(egui::Label::new(job).wrap());
        });
}

/// Escala un tamaño de fuente base por el factor global del usuario
/// (1.0 = tamaño original). Se usa en toda la interfaz, no solo en Markdown.
pub fn scaled_size(base: f32, scale: f32) -> f32 {
    base * scale
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
        append_inline(job, inline, fmt.clone());
    }
}

fn append_inline(job: &mut LayoutJob, inline: &Inline, fmt: TextFormat) {
    match inline {
        Inline::Text(t) => job.append(t, 0.0, fmt),
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
            // Ruta estática: enlaces coloreados/subrayados dentro de un LayoutJob.
            let mut f = fmt.clone();
            f.color = Color32::from_rgb(0x62, 0x9c, 0xf0);
            f.underline = egui::Stroke::new(1.0_f32, f.color);
            append_inlines(job, inner, f);
            let _ = url;
        }
        Inline::Break => {
            job.append("\n", 0.0, fmt);
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

    #[test]
    fn parses_table_with_headers_and_rows() {
        let blocks = parse("| Nombre | Edad |\n|---|---|\n| Ana | 30 |\n| Luis | 25 |");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Table { headers, rows } => {
                assert_eq!(headers.len(), 2);
                assert_eq!(inline_plain(&headers[0]), "Nombre");
                assert_eq!(inline_plain(&headers[1]), "Edad");
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].len(), 2);
                assert_eq!(inline_plain(&rows[0][0]), "Ana");
                assert_eq!(inline_plain(&rows[0][1]), "30");
                assert_eq!(inline_plain(&rows[1][0]), "Luis");
                assert_eq!(inline_plain(&rows[1][1]), "25");
            }
            other => panic!("esperaba tabla, got {other:?}"),
        }
    }

    #[test]
    fn parses_clickable_link() {
        let blocks = parse("Mira [este enlace](https://example.com) y sigue.");
        match &blocks[0] {
            Block::Para(inlines) => {
                assert!(matches!(&inlines[1], Inline::Link(url, _) if url == "https://example.com"));
            }
            other => panic!("esperaba párrafo, got {other:?}"),
        }
    }
}

