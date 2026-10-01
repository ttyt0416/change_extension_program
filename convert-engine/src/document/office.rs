//! DOCX/HWP/HWPX conversion uses an internal document model; no office installation is needed.
mod hwp;
mod hwpx;

use crate::Error;
use roxmltree::{Document, Node};
use std::io::{Cursor, Read};
use std::path::Path;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const MAX_STREAM: u64 = 64 * 1024 * 1024;
pub(crate) const HWP_NOTICE: &str =
    "본 제품은 한컴의 HWP 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.";

#[derive(Clone, Debug, PartialEq, Eq)]
struct TextStyle {
    font: String,
    size: u32, // half points
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    color: u32, // RGB
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: "맑은 고딕".into(),
            size: 22,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TextRun {
    text: String,
    style: TextStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParagraphStyle {
    align: u8, // HWP: 0 justified, 1 left, 2 right, 3 center, 4 distribute
    before: u32,
    after: u32,
    left: i32,
    right: i32,
    indent: i32, // twips
    page_break: bool,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        Self {
            align: 1,
            before: 0,
            after: 0,
            left: 0,
            right: 0,
            indent: 0,
            page_break: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Paragraph {
    runs: Vec<TextRun>,
    style: ParagraphStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Cell {
    row: usize,
    col: usize,
    row_span: usize,
    col_span: usize,
    width: u32, // twips
    paragraphs: Vec<Paragraph>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Table {
    rows: usize,
    cols: usize,
    cells: Vec<Cell>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Page {
    width: u32,
    height: u32,
    margins: [u32; 6],
} // L,R,T,B,header,footer; twips

impl Default for Page {
    fn default() -> Self {
        Self {
            width: 11906,
            height: 16838,
            margins: [1440, 1440, 1440, 1440, 720, 720],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OfficeDocument {
    blocks: Vec<Block>,
    page: Page,
}

fn error(message: impl Into<String>) -> Error {
    Error::Document(message.into())
}

fn limited_read(reader: impl Read) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    reader.take(MAX_STREAM + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STREAM {
        return Err(error("문서 데이터가 너무 큽니다"));
    }
    Ok(bytes)
}

pub(super) fn docx_supported(path: &Path) -> bool {
    read_docx(path).is_ok()
}
pub(super) fn hwp_supported(path: &Path) -> bool {
    hwp::read(path).is_ok()
}

pub(super) fn docx_to_hwp(path: &Path) -> Result<Vec<u8>, Error> {
    hwp::write(&read_docx(path)?)
}

pub(super) fn hwp_to_docx(path: &Path) -> Result<Vec<u8>, Error> {
    write_docx(&hwp::read(path)?)
}

pub(super) fn hwpx_supported(path: &Path) -> bool {
    hwpx::read(path).is_ok()
}

pub(super) fn docx_to_hwpx(path: &Path) -> Result<Vec<u8>, Error> {
    hwpx::write(&read_docx(path)?)
}

pub(super) fn hwpx_to_docx(path: &Path) -> Result<Vec<u8>, Error> {
    write_docx(&hwpx::read(path)?)
}

pub(super) fn hwp_to_hwpx(path: &Path) -> Result<Vec<u8>, Error> {
    hwpx::write(&hwp::read(path)?)
}

pub(super) fn hwpx_to_hwp(path: &Path) -> Result<Vec<u8>, Error> {
    hwp::write(&hwpx::read(path)?)
}

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|node| node.has_tag_name((W, name)))
}
fn attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attribute((W, name))
}
fn value<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(node, name).and_then(|n| attr(n, "val"))
}
fn number(node: Node<'_, '_>, name: &str, fallback: u32) -> u32 {
    attr(node, name)
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}
fn enabled(node: Node<'_, '_>) -> bool {
    !matches!(attr(node, "val"), Some("0" | "false" | "off"))
}

fn apply_run_properties(style: &mut TextStyle, props: Node<'_, '_>) {
    if let Some(fonts) = child(props, "rFonts") {
        if let Some(font) = attr(fonts, "eastAsia").or_else(|| attr(fonts, "ascii")) {
            style.font = font.into();
        }
    }
    if let Some(size) = value(props, "sz").and_then(|v| v.parse::<u32>().ok()) {
        style.size = size.clamp(1, 8192);
    }
    for (name, field) in [
        ("b", &mut style.bold),
        ("i", &mut style.italic),
        ("strike", &mut style.strike),
    ] {
        if let Some(node) = child(props, name) {
            *field = enabled(node);
        }
    }
    if let Some(node) = child(props, "u") {
        style.underline = !matches!(attr(node, "val"), Some("none" | "0" | "false"));
    }
    if let Some(color) = value(props, "color").and_then(|v| u32::from_str_radix(v, 16).ok()) {
        style.color = color & 0xffffff;
    }
}

fn apply_paragraph_properties(
    style: &mut ParagraphStyle,
    props: Node<'_, '_>,
) -> Result<(), Error> {
    if child(props, "numPr").is_some() {
        return Err(error(
            "DOCX↔HWP 변환은 자동 번호·글머리표를 아직 지원하지 않습니다",
        ));
    }
    if let Some(align) = value(props, "jc") {
        style.align = match align {
            "center" => 3,
            "right" | "end" => 2,
            "both" | "justified" => 0,
            "distribute" => 4,
            _ => 1,
        };
    }
    if let Some(spacing) = child(props, "spacing") {
        style.before = number(spacing, "before", style.before);
        style.after = number(spacing, "after", style.after);
    }
    if let Some(indent) = child(props, "ind") {
        style.left = attr(indent, "left")
            .or_else(|| attr(indent, "start"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(style.left);
        style.right = attr(indent, "right")
            .or_else(|| attr(indent, "end"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(style.right);
        if let Some(v) = attr(indent, "firstLine").and_then(|v| v.parse().ok()) {
            style.indent = v;
        }
        if let Some(v) = attr(indent, "hanging").and_then(|v| v.parse::<i32>().ok()) {
            style.indent = -v;
        }
    }
    if let Some(node) = child(props, "pageBreakBefore") {
        style.page_break = enabled(node);
    }
    Ok(())
}

fn apply_named_style(
    styles: &Document<'_>,
    name: &str,
    run: &mut TextStyle,
    para: &mut ParagraphStyle,
    chain: &mut Vec<String>,
) -> Result<(), Error> {
    if chain.iter().any(|id| id == name) || chain.len() >= 32 {
        return Err(error("DOCX 스타일 참조가 올바르지 않습니다"));
    }
    let Some(style) = styles
        .descendants()
        .find(|n| n.has_tag_name((W, "style")) && attr(*n, "styleId") == Some(name))
    else {
        return Ok(());
    };
    chain.push(name.into());
    if let Some(parent) = value(style, "basedOn") {
        apply_named_style(styles, parent, run, para, chain)?;
    }
    if let Some(props) = child(style, "rPr") {
        apply_run_properties(run, props);
    }
    if let Some(props) = child(style, "pPr") {
        apply_paragraph_properties(para, props)?;
    }
    chain.pop();
    Ok(())
}

fn parse_paragraph(node: Node<'_, '_>, styles: &Document<'_>) -> Result<Paragraph, Error> {
    let mut paragraph = Paragraph::default();
    let mut base = TextStyle::default();
    if let Some(defaults) = styles
        .descendants()
        .find(|n| n.has_tag_name((W, "docDefaults")))
    {
        if let Some(props) = child(defaults, "rPrDefault").and_then(|n| child(n, "rPr")) {
            apply_run_properties(&mut base, props);
        }
        if let Some(props) = child(defaults, "pPrDefault").and_then(|n| child(n, "pPr")) {
            apply_paragraph_properties(&mut paragraph.style, props)?;
        }
    }
    let props = child(node, "pPr");
    let default_style = styles
        .descendants()
        .find(|n| {
            n.has_tag_name((W, "style"))
                && attr(*n, "type") == Some("paragraph")
                && attr(*n, "default") == Some("1")
        })
        .and_then(|n| attr(n, "styleId"));
    if let Some(name) = props.and_then(|n| value(n, "pStyle")).or(default_style) {
        apply_named_style(
            styles,
            name,
            &mut base,
            &mut paragraph.style,
            &mut Vec::new(),
        )?;
    }
    if let Some(props) = props {
        apply_paragraph_properties(&mut paragraph.style, props)?;
    }
    for run in node.descendants().filter(|n| n.has_tag_name((W, "r"))) {
        let mut style = base.clone();
        if let Some(props) = child(run, "rPr") {
            if let Some(name) = value(props, "rStyle") {
                apply_named_style(
                    styles,
                    name,
                    &mut style,
                    &mut paragraph.style.clone(),
                    &mut Vec::new(),
                )?;
            }
            apply_run_properties(&mut style, props);
        }
        let mut text = String::new();
        for item in run.children().filter(|n| n.is_element()) {
            match item.tag_name().name() {
                "t" => text.push_str(item.text().unwrap_or_default()),
                "tab" => text.push('\t'),
                "br" if !matches!(attr(item, "type"), Some("page" | "column")) => text.push('\n'),
                "cr" => text.push('\n'),
                "noBreakHyphen" => text.push('\u{2011}'),
                "softHyphen" => text.push('\u{ad}'),
                "rPr" | "lastRenderedPageBreak" => {}
                _ => {
                    return Err(error(format!(
                        "DOCX↔HWP 변환에서 지원하지 않는 문서 요소입니다: {}",
                        item.tag_name().name()
                    )));
                }
            }
        }
        if !text.is_empty() {
            paragraph.runs.push(TextRun { text, style });
        }
    }
    Ok(paragraph)
}

fn parse_table(node: Node<'_, '_>, styles: &Document<'_>) -> Result<Table, Error> {
    let rows: Vec<_> = node
        .children()
        .filter(|n| n.has_tag_name((W, "tr")))
        .collect();
    let mut table = Table {
        rows: rows.len(),
        cols: 0,
        cells: Vec::new(),
    };
    for (row, node) in rows.into_iter().enumerate() {
        let mut col = 0;
        for cell in node.children().filter(|n| n.has_tag_name((W, "tc"))) {
            let props = child(cell, "tcPr");
            let col_span = props
                .and_then(|n| value(n, "gridSpan"))
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            if col_span == 0 || col_span > 256 {
                return Err(error("DOCX 표의 셀 병합 정보가 올바르지 않습니다"));
            }
            let width = props
                .and_then(|n| child(n, "tcW"))
                .map(|n| number(n, "w", 2000))
                .unwrap_or(2000);
            let merge = props.and_then(|n| child(n, "vMerge"));
            let mut paragraphs = Vec::new();
            for p in cell.children().filter(|n| n.is_element()) {
                if p.has_tag_name((W, "p")) {
                    paragraphs.push(parse_paragraph(p, styles)?);
                } else if !p.has_tag_name((W, "tcPr")) {
                    return Err(error(
                        "중첩 표 등 복합 셀은 DOCX↔HWP 변환에서 지원하지 않습니다",
                    ));
                }
            }
            if merge.is_some_and(|n| attr(n, "val") != Some("restart")) {
                if paragraphs.iter().any(|p| !p.runs.is_empty()) {
                    return Err(error("병합된 DOCX 셀에 추가 텍스트가 있습니다"));
                }
                let parent = table
                    .cells
                    .iter_mut()
                    .rev()
                    .find(|c| c.col == col && c.col_span == col_span && c.row + c.row_span == row)
                    .ok_or_else(|| error("DOCX 세로 병합 정보가 올바르지 않습니다"))?;
                parent.row_span += 1;
            } else {
                if paragraphs.is_empty() {
                    paragraphs.push(Paragraph::default());
                }
                table.cells.push(Cell {
                    row,
                    col,
                    row_span: 1,
                    col_span,
                    width,
                    paragraphs,
                });
            }
            col += col_span;
        }
        table.cols = table.cols.max(col);
    }
    validate_table(&table)?;
    Ok(table)
}

fn validate_table(table: &Table) -> Result<(), Error> {
    if table.rows == 0 || table.cols == 0 || table.rows * table.cols > 10000 {
        return Err(error("지원하지 않는 표 크기입니다"));
    }
    let mut occupied = vec![false; table.rows * table.cols];
    for cell in &table.cells {
        if cell.row_span == 0
            || cell.col_span == 0
            || cell.row + cell.row_span > table.rows
            || cell.col + cell.col_span > table.cols
        {
            return Err(error("표의 셀 범위가 올바르지 않습니다"));
        }
        for row in cell.row..cell.row + cell.row_span {
            for col in cell.col..cell.col + cell.col_span {
                let slot = &mut occupied[row * table.cols + col];
                if *slot {
                    return Err(error("표의 셀 범위가 겹칩니다"));
                }
                *slot = true;
            }
        }
    }
    if occupied.contains(&false) {
        return Err(error("표에 누락된 셀이 있습니다"));
    }
    Ok(())
}

fn read_docx(path: &Path) -> Result<OfficeDocument, Error> {
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(path)?).map_err(|e| error(e.to_string()))?;
    let xml = limited_read(
        archive
            .by_name("word/document.xml")
            .map_err(|e| error(e.to_string()))?,
    )?;
    let styles = match archive.by_name("word/styles.xml") {
        Ok(entry) => limited_read(entry)?,
        Err(zip::result::ZipError::FileNotFound) => {
            b"<w:styles xmlns:w='http://schemas.openxmlformats.org/wordprocessingml/2006/main'/>"
                .to_vec()
        }
        Err(e) => return Err(error(e.to_string())),
    };
    let document = Document::parse(std::str::from_utf8(&xml).map_err(|e| error(e.to_string()))?)
        .map_err(|e| error(e.to_string()))?;
    let styles = Document::parse(std::str::from_utf8(&styles).map_err(|e| error(e.to_string()))?)
        .map_err(|e| error(e.to_string()))?;
    // Reject content we cannot transfer, rather than silently creating an incomplete document.
    for node in document.descendants().filter(|n| n.is_element()) {
        if matches!(
            node.tag_name().name(),
            "drawing"
                | "pict"
                | "object"
                | "oMath"
                | "oMathPara"
                | "footnoteReference"
                | "endnoteReference"
                | "headerReference"
                | "footerReference"
                | "altChunk"
                | "ins"
                | "del"
        ) {
            return Err(error(
                "DOCX↔HWP 변환은 그림·수식·각주·머리말/꼬리말·변경 추적을 아직 지원하지 않습니다",
            ));
        }
    }
    let body = document
        .descendants()
        .find(|n| n.has_tag_name((W, "body")))
        .ok_or_else(|| error("DOCX 본문이 없습니다"))?;
    let mut result = OfficeDocument::default();
    for node in body.children().filter(|n| n.is_element()) {
        if node.has_tag_name((W, "p")) {
            if child(node, "pPr")
                .and_then(|n| child(n, "sectPr"))
                .is_some()
            {
                return Err(error("여러 구역으로 구성된 DOCX는 아직 지원하지 않습니다"));
            }
            result
                .blocks
                .push(Block::Paragraph(parse_paragraph(node, &styles)?));
        } else if node.has_tag_name((W, "tbl")) {
            result
                .blocks
                .push(Block::Table(parse_table(node, &styles)?));
        } else if node.has_tag_name((W, "sectPr")) {
            if let Some(size) = child(node, "pgSz") {
                result.page.width = number(size, "w", result.page.width);
                result.page.height = number(size, "h", result.page.height);
            }
            if let Some(margin) = child(node, "pgMar") {
                for (i, name) in ["left", "right", "top", "bottom", "header", "footer"]
                    .iter()
                    .enumerate()
                {
                    result.page.margins[i] = number(margin, name, result.page.margins[i]);
                }
            }
            if child(node, "cols").is_some_and(|n| number(n, "num", 1) > 1) {
                return Err(error("다단 DOCX는 아직 지원하지 않습니다"));
            }
        } else {
            return Err(error(format!(
                "지원하지 않는 DOCX 본문 요소입니다: {}",
                node.tag_name().name()
            )));
        }
    }
    if result.blocks.is_empty() {
        result.blocks.push(Block::Paragraph(Paragraph::default()));
    }
    validate_document(&result)?;
    Ok(result)
}

fn validate_page(page: &Page) -> Result<(), Error> {
    if page.width == 0
        || page.height == 0
        || page.width > 100000
        || page.height > 100000
        || page.margins[0]
            .checked_add(page.margins[1])
            .is_none_or(|n| n >= page.width)
        || page.margins[2]
            .checked_add(page.margins[3])
            .is_none_or(|n| n >= page.height)
    {
        return Err(error("문서의 용지 크기 또는 여백이 올바르지 않습니다"));
    }
    Ok(())
}

fn validate_document(document: &OfficeDocument) -> Result<(), Error> {
    validate_page(&document.page)?;
    let validate_paragraph = |p: &Paragraph| {
        let s = &p.style;
        if s.before > 100000
            || s.after > 100000
            || [s.left, s.right, s.indent]
                .iter()
                .any(|n| n.unsigned_abs() > 100000)
            || p.runs.len() > 65535
        {
            return Err(error("문단 서식 값이 지원 범위를 벗어났습니다"));
        }
        Ok(())
    };
    for block in &document.blocks {
        match block {
            Block::Paragraph(p) => validate_paragraph(p)?,
            Block::Table(table) => {
                validate_table(table)?;
                for cell in &table.cells {
                    if cell.width == 0
                        || cell.width > 100000
                        || cell.paragraphs.is_empty()
                        || cell.paragraphs.len() > 10000
                    {
                        return Err(error("표의 셀 크기 또는 문단 수가 올바르지 않습니다"));
                    }
                    for p in &cell.paragraphs {
                        validate_paragraph(p)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn docx_paragraph(paragraph: &Paragraph) -> docx_rs::Paragraph {
    use docx_rs::*;
    let style = &paragraph.style;
    let alignment = match style.align {
        0 => AlignmentType::Both,
        2 => AlignmentType::Right,
        3 => AlignmentType::Center,
        4 | 5 => AlignmentType::Distribute,
        _ => AlignmentType::Left,
    };
    let mut output = docx_rs::Paragraph::new()
        .align(alignment)
        .line_spacing(LineSpacing::new().before(style.before).after(style.after))
        .indent(
            Some(style.left),
            Some(if style.indent >= 0 {
                SpecialIndentType::FirstLine(style.indent)
            } else {
                SpecialIndentType::Hanging(-style.indent)
            }),
            Some(style.right),
            None,
        );
    if style.page_break {
        output = output.page_break_before(true);
    }
    for item in &paragraph.runs {
        let s = &item.style;
        let mut run = Run::new()
            .size(s.size as usize)
            .color(format!("{:06X}", s.color))
            .fonts(
                RunFonts::new()
                    .ascii(&s.font)
                    .east_asia(&s.font)
                    .hi_ansi(&s.font),
            );
        if s.bold {
            run = run.bold();
        }
        if s.italic {
            run = run.italic();
        }
        if s.underline {
            run = run.underline("single");
        }
        if s.strike {
            run = run.strike();
        }
        let mut text = String::new();
        for ch in item.text.chars() {
            if ch == '\t' || ch == '\n' {
                if !text.is_empty() {
                    run = run.add_text(std::mem::take(&mut text));
                }
                run = if ch == '\t' {
                    run.add_tab()
                } else {
                    run.add_break(BreakType::TextWrapping)
                };
            } else {
                text.push(ch);
            }
        }
        if !text.is_empty() {
            run = run.add_text(text);
        }
        output = output.add_run(run);
    }
    output
}

fn write_docx(document: &OfficeDocument) -> Result<Vec<u8>, Error> {
    use docx_rs::*;
    let m = document.page.margins;
    let mut output = Docx::new()
        .page_size(document.page.width, document.page.height)
        .page_margin(
            PageMargin::new()
                .left(m[0] as i32)
                .right(m[1] as i32)
                .top(m[2] as i32)
                .bottom(m[3] as i32)
                .header(m[4] as i32)
                .footer(m[5] as i32),
        );
    for block in &document.blocks {
        match block {
            Block::Paragraph(p) => output = output.add_paragraph(docx_paragraph(p)),
            Block::Table(table) => {
                let mut rows = Vec::new();
                for row in 0..table.rows {
                    let mut cells = Vec::new();
                    let mut col = 0;
                    while col < table.cols {
                        let cell = table
                            .cells
                            .iter()
                            .find(|c| c.col == col && c.row <= row && c.row + c.row_span > row)
                            .ok_or_else(|| error("표의 셀 정보가 올바르지 않습니다"))?;
                        let mut out = TableCell::new()
                            .grid_span(cell.col_span)
                            .width(cell.width as usize, WidthType::Dxa);
                        if cell.row_span > 1 {
                            out = out.vertical_merge(if cell.row == row {
                                VMergeType::Restart
                            } else {
                                VMergeType::Continue
                            });
                        }
                        if cell.row == row {
                            for p in &cell.paragraphs {
                                out = out.add_paragraph(docx_paragraph(p));
                            }
                        } else {
                            out = out.add_paragraph(docx_rs::Paragraph::new());
                        }
                        cells.push(out);
                        col += cell.col_span;
                    }
                    rows.push(TableRow::new(cells));
                }
                output = output.add_table(docx_rs::Table::new(rows));
            }
        }
    }
    let mut bytes = Cursor::new(Vec::new());
    output
        .build()
        .pack(&mut bytes)
        .map_err(|e| error(e.to_string()))?;
    Ok(bytes.into_inner())
}

#[cfg(test)]
mod tests;
