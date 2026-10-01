//! 본 제품은 한컴의 HWP 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
//! HWP 5.0 revision 1.2: https://www.hancom.com/support/downloadCenter/hwpOwpml
use super::*;
use cfb::CompoundFile;
use flate2::{Compression, read::DeflateDecoder, write::DeflateEncoder};
use std::io::Write;

const PARA_HEADER: u16 = 0x42;
const PARA_TEXT: u16 = 0x43;
const CHAR_POS: u16 = 0x44;
const CTRL: u16 = 0x47;
const LIST: u16 = 0x48;

#[derive(Debug)]
struct Record<'a> {
    tag: u16,
    level: u16,
    data: &'a [u8],
}

fn u16_at(data: &[u8], offset: usize) -> Result<u16, Error> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| error("HWP 데이터가 잘렸습니다"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32, Error> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| error("HWP 데이터가 잘렸습니다"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
fn utf16(data: &[u8]) -> Result<String, Error> {
    if data.len() % 2 != 0 {
        return Err(error("HWP 문자열 길이가 올바르지 않습니다"));
    }
    String::from_utf16(
        &data
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>(),
    )
    .map_err(|e| error(e.to_string()))
}

fn records(data: &[u8]) -> Result<Vec<Record<'_>>, Error> {
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let header = u32_at(data, offset)?;
        offset += 4;
        let mut size = (header >> 20) as usize;
        if size == 0xfff {
            size = u32_at(data, offset)? as usize;
            offset += 4;
        }
        let bytes = data
            .get(offset..offset + size)
            .ok_or_else(|| error("HWP 레코드 길이가 올바르지 않습니다"))?;
        result.push(Record {
            tag: (header & 0x3ff) as u16,
            level: ((header >> 10) & 0x3ff) as u16,
            data: bytes,
        });
        offset += size;
    }
    Ok(result)
}

fn stream<R: Read + std::io::Seek>(
    file: &mut CompoundFile<R>,
    name: &str,
    compressed: bool,
) -> Result<Vec<u8>, Error> {
    let bytes = limited_read(file.open_stream(name).map_err(|e| error(e.to_string()))?)?;
    if compressed {
        limited_read(DeflateDecoder::new(bytes.as_slice()))
    } else {
        Ok(bytes)
    }
}

struct DocInfo {
    text: Vec<TextStyle>,
    para: Vec<(ParagraphStyle, bool)>,
}

fn parse_info(data: &[u8]) -> Result<DocInfo, Error> {
    let records = records(data)?;
    let fonts: Vec<_> = records
        .iter()
        .filter(|r| r.tag == 0x13)
        .map(|r| {
            let len = u16_at(r.data, 1)? as usize;
            utf16(
                r.data
                    .get(3..3 + len * 2)
                    .ok_or_else(|| error("HWP 글꼴 정보가 올바르지 않습니다"))?,
            )
        })
        .collect::<Result<_, Error>>()?;
    let mut info = DocInfo {
        text: Vec::new(),
        para: Vec::new(),
    };
    for record in &records {
        if record.tag == 0x15 {
            let data = record.data;
            let props = u32_at(data, 46)?;
            let font_id = u16_at(data, 0)? as usize;
            let size = u32_at(data, 42)? as i32;
            if size <= 0 || size > 409600 {
                return Err(error("HWP 글자 크기가 올바르지 않습니다"));
            }
            let color = u32_at(data, 52)?;
            info.text.push(TextStyle {
                font: fonts
                    .get(font_id)
                    .cloned()
                    .unwrap_or_else(|| "맑은 고딕".into()),
                size: (size as u32 + 25) / 50,
                bold: props & 2 != 0,
                italic: props & 1 != 0,
                underline: props & 0xc != 0,
                strike: props & (7 << 18) != 0,
                color: colorref_to_rgb(color),
            });
        } else if record.tag == 0x19 {
            let data = record.data;
            let props = u32_at(data, 0)?;
            info.para.push((
                ParagraphStyle {
                    align: ((props >> 2) & 7) as u8,
                    left: (u32_at(data, 4)? as i32) / 5,
                    right: (u32_at(data, 8)? as i32) / 5,
                    indent: (u32_at(data, 12)? as i32) / 5,
                    before: (u32_at(data, 16)? as i32).max(0) as u32 / 5,
                    after: (u32_at(data, 20)? as i32).max(0) as u32 / 5,
                    page_break: props & (1 << 19) != 0,
                },
                props & (3 << 23) != 0,
            ));
        }
    }
    if info.text.is_empty() || info.para.is_empty() {
        return Err(error("HWP 문서에 글자·문단 정보가 없습니다"));
    }
    Ok(info)
}

fn end_of(records: &[Record<'_>], start: usize) -> usize {
    let level = records[start].level;
    (start + 1..records.len())
        .find(|&i| records[i].level <= level)
        .unwrap_or(records.len())
}

fn parse_text(
    data: &[u8],
    shapes: &[(usize, usize)],
    info: &DocInfo,
) -> Result<Vec<TextRun>, Error> {
    if data.len() % 2 != 0 {
        return Err(error("HWP 본문 문자열 길이가 올바르지 않습니다"));
    }
    let units: Vec<_> = data
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let mut runs: Vec<TextRun> = Vec::new();
    let mut index = 0;
    let mut shape_index = 0;
    while index < units.len() {
        while shape_index + 1 < shapes.len() && shapes[shape_index + 1].0 <= index {
            shape_index += 1;
        }
        let style = info
            .text
            .get(shapes[shape_index].1)
            .ok_or_else(|| error("HWP 글자 모양 참조가 올바르지 않습니다"))?
            .clone();
        let code = units[index];
        let mut consumed = 1;
        let text = match code {
            0x0d => String::new(),
            0x0a => "\n".to_owned(),
            0x09 => {
                consumed = 8;
                "\t".to_owned()
            }
            0x01..=0x08 | 0x0b..=0x0c | 0x0e..=0x17 => {
                consumed = 8;
                String::new()
            }
            0x00 | 0x18..=0x1f => String::new(),
            0xd800..=0xdbff => {
                consumed = 2;
                String::from_utf16(
                    units
                        .get(index..index + 2)
                        .ok_or_else(|| error("잘린 HWP 유니코드 문자입니다"))?,
                )
                .map_err(|e| error(e.to_string()))?
            }
            _ => String::from_utf16(&[code]).map_err(|e| error(e.to_string()))?,
        };
        if index + consumed > units.len() {
            return Err(error("HWP 제어 문자가 잘렸습니다"));
        }
        if !text.is_empty() {
            if let Some(last) = runs.last_mut().filter(|last| last.style == style) {
                last.text.push_str(&text);
            } else {
                runs.push(TextRun { text, style });
            }
        }
        index += consumed;
    }
    Ok(runs)
}

fn parse_paragraph(
    records: &[Record<'_>],
    start: usize,
    info: &DocInfo,
) -> Result<(Paragraph, Vec<Table>, usize), Error> {
    let end = end_of(records, start);
    let header = records[start].data;
    let para_id = u16_at(header, 8)? as usize;
    let (style, numbering) = info
        .para
        .get(para_id)
        .ok_or_else(|| error("HWP 문단 모양 참조가 올바르지 않습니다"))?;
    if *numbering {
        return Err(error(
            "DOCX↔HWP 변환은 자동 번호·글머리표를 아직 지원하지 않습니다",
        ));
    }
    let mut paragraph = Paragraph {
        runs: Vec::new(),
        style: style.clone(),
    };
    paragraph.style.page_break |= header
        .get(11)
        .ok_or_else(|| error("잘린 HWP 문단 헤더입니다"))?
        & 4
        != 0;
    let children = &records[start + 1..end];
    let text = children
        .iter()
        .find(|r| r.level == records[start].level + 1 && r.tag == PARA_TEXT)
        .map(|r| r.data)
        .unwrap_or_default();
    let shapes: Vec<_> = if let Some(record) = children
        .iter()
        .find(|r| r.level == records[start].level + 1 && r.tag == CHAR_POS)
    {
        if record.data.len() % 8 != 0 {
            return Err(error("HWP 글자 위치 정보가 올바르지 않습니다"));
        }
        record
            .data
            .chunks_exact(8)
            .map(|b| Ok((u32_at(b, 0)? as usize, u32_at(b, 4)? as usize)))
            .collect::<Result<_, Error>>()?
    } else {
        vec![(0, 0)]
    };
    if shapes.is_empty() || shapes[0].0 != 0 || shapes.windows(2).any(|p| p[0].0 >= p[1].0) {
        return Err(error("HWP 글자 위치가 올바르지 않습니다"));
    }
    paragraph.runs = parse_text(text, &shapes, info)?;
    let mut tables = Vec::new();
    let mut i = start + 1;
    while i < end {
        let record = &records[i];
        if record.tag == CTRL && record.level == records[start].level + 1 {
            let ctrl_id = record
                .data
                .get(..4)
                .ok_or_else(|| error("잘린 HWP 컨트롤입니다"))?;
            match ctrl_id {
                b" lbt" => tables.push(parse_table(records, i, info)?),
                b"dces" | b"dloc" => {}
                _ => {
                    return Err(error(
                        "DOCX↔HWP 변환은 그림·수식·글상자·각주·머리말/꼬리말 등 복합 개체를 아직 지원하지 않습니다",
                    ));
                }
            }
            i = end_of(records, i);
        } else {
            i += 1;
        }
    }
    if !tables.is_empty() && !paragraph.runs.is_empty() {
        return Err(error(
            "본문과 표가 한 문단에 섞인 HWP는 아직 지원하지 않습니다",
        ));
    }
    Ok((paragraph, tables, end))
}

fn parse_table(records: &[Record<'_>], start: usize, info: &DocInfo) -> Result<Table, Error> {
    let end = end_of(records, start);
    let level = records[start].level + 1;
    let props = records[start + 1..end]
        .iter()
        .find(|r| r.tag == 0x4d && r.level == level)
        .ok_or_else(|| error("HWP 표 속성이 없습니다"))?;
    let mut table = Table {
        rows: u16_at(props.data, 4)? as usize,
        cols: u16_at(props.data, 6)? as usize,
        cells: Vec::new(),
    };
    if table.rows == 0 || table.cols == 0 || table.rows * table.cols > 10000 {
        return Err(error("지원하지 않는 HWP 표 크기입니다"));
    }
    let mut i = start + 1;
    while i < end {
        if records[i].tag != LIST || records[i].level != level {
            i += 1;
            continue;
        }
        let data = records[i].data;
        if data.len() < 34 {
            return Err(error("HWP 표 캡션 등 복합 표는 아직 지원하지 않습니다"));
        }
        let count = u32_at(data, 0)? as usize;
        if count == 0 || count > 10000 {
            return Err(error("HWP 셀 문단 수가 올바르지 않습니다"));
        }
        let mut cell = Cell {
            col: u16_at(data, 8)? as usize,
            row: u16_at(data, 10)? as usize,
            col_span: u16_at(data, 12)? as usize,
            row_span: u16_at(data, 14)? as usize,
            width: u32_at(data, 16)? / 5,
            paragraphs: Vec::new(),
        };
        i += 1;
        for _ in 0..count {
            if i >= end || records[i].tag != PARA_HEADER || records[i].level != level {
                return Err(error("HWP 셀 문단 정보가 올바르지 않습니다"));
            }
            let (paragraph, nested, next) = parse_paragraph(records, i, info)?;
            if !nested.is_empty() {
                return Err(error("중첩 표는 DOCX↔HWP 변환에서 지원하지 않습니다"));
            }
            cell.paragraphs.push(paragraph);
            i = next;
        }
        table.cells.push(cell);
    }
    validate_table(&table)?;
    Ok(table)
}

pub(super) fn read(path: &Path) -> Result<OfficeDocument, Error> {
    let mut file = cfb::open(path).map_err(|e| error(e.to_string()))?;
    let header = stream(&mut file, "/FileHeader", false)?;
    if header.len() < 256 || !header.starts_with(b"HWP Document File\0") || header[35] != 5 {
        return Err(error("HWP 5.0 문서만 지원합니다"));
    }
    let flags = u32_at(&header, 36)?;
    if flags & (2 | 4 | 16 | 256 | 1024) != 0 {
        return Err(error("암호·배포 제한·DRM이 적용된 HWP는 지원하지 않습니다"));
    }
    let compressed = flags & 1 != 0;
    let info = parse_info(&stream(&mut file, "/DocInfo", compressed)?)?;
    let mut sections: Vec<_> = file
        .walk()
        .filter_map(|entry| {
            entry
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|_| {
                    entry.is_stream() && entry.path().parent() == Some(Path::new("/BodyText"))
                })
                .and_then(|name| name.strip_prefix("Section"))
                .and_then(|number| number.parse::<usize>().ok())
        })
        .collect();
    sections.sort_unstable();
    if sections.is_empty()
        || sections.len() > 1024
        || sections.iter().enumerate().any(|(i, &number)| number != i)
    {
        return Err(error("HWP 구역 정보가 올바르지 않습니다"));
    }
    let mut result = OfficeDocument::default();
    for (section_index, section) in sections.into_iter().enumerate() {
        let bytes = stream(
            &mut file,
            &format!("/BodyText/Section{section}"),
            compressed,
        )?;
        let records = records(&bytes)?;
        if let Some(page) = records.iter().find(|r| r.tag == 0x49) {
            let mut parsed = Page {
                width: u32_at(page.data, 0)? / 5,
                height: u32_at(page.data, 4)? / 5,
                ..Page::default()
            };
            for i in 0..6 {
                parsed.margins[i] = u32_at(page.data, 8 + i * 4)? / 5;
            }
            if u32_at(page.data, 36)? & 1 != 0 {
                std::mem::swap(&mut parsed.width, &mut parsed.height);
            }
            validate_page(&parsed)?;
            if section_index == 0 {
                result.page = parsed;
            } else if parsed != result.page {
                return Err(error(
                    "구역별 용지 설정이 다른 HWP는 아직 지원하지 않습니다",
                ));
            }
        }
        let mut i = 0;
        let mut first = true;
        while i < records.len() {
            if records[i].tag != PARA_HEADER || records[i].level != 0 {
                return Err(error("HWP 본문 구조가 올바르지 않습니다"));
            }
            let (mut paragraph, tables, next) = parse_paragraph(&records, i, &info)?;
            if section_index > 0 && first {
                paragraph.style.page_break = true;
            }
            if tables.is_empty() {
                result.blocks.push(Block::Paragraph(paragraph));
            } else {
                for table in tables {
                    result.blocks.push(Block::Table(table));
                }
            }
            first = false;
            i = next;
        }
    }
    if result.blocks.is_empty() {
        return Err(error("HWP 본문이 없습니다"));
    }
    validate_document(&result)?;
    Ok(result)
}

fn put16(data: &mut Vec<u8>, value: u16) {
    data.extend_from_slice(&value.to_le_bytes());
}
fn put32(data: &mut Vec<u8>, value: u32) {
    data.extend_from_slice(&value.to_le_bytes());
}
fn put_text(data: &mut Vec<u8>, text: &str) {
    for unit in text.encode_utf16() {
        put16(data, unit);
    }
}
fn sized_text(data: &mut Vec<u8>, text: &str) {
    put16(data, text.encode_utf16().count() as u16);
    put_text(data, text);
}
fn record(output: &mut Vec<u8>, tag: u16, level: u16, data: &[u8]) {
    let size = data.len() as u32;
    put32(
        output,
        tag as u32 | ((level as u32) << 10) | (size.min(0xfff) << 20),
    );
    if size >= 0xfff {
        put32(output, size);
    }
    output.extend_from_slice(data);
}
fn colorref_to_rgb(color: u32) -> u32 {
    ((color & 0xff) << 16) | (color & 0xff00) | ((color >> 16) & 0xff)
}
fn rgb_to_colorref(color: u32) -> u32 {
    colorref_to_rgb(color)
}

struct Styles {
    text: Vec<TextStyle>,
    para: Vec<ParagraphStyle>,
    fonts: Vec<String>,
}
impl Styles {
    fn collect(document: &OfficeDocument) -> Result<Self, Error> {
        let mut result = Self {
            text: vec![TextStyle::default()],
            para: vec![ParagraphStyle::default()],
            fonts: vec![TextStyle::default().font],
        };
        let mut collect = |p: &Paragraph| {
            if !result.para.contains(&p.style) {
                result.para.push(p.style.clone());
            }
            for run in &p.runs {
                if !result.text.contains(&run.style) {
                    result.text.push(run.style.clone());
                }
                if !result.fonts.contains(&run.style.font) {
                    result.fonts.push(run.style.font.clone());
                }
            }
        };
        for block in &document.blocks {
            match block {
                Block::Paragraph(p) => collect(p),
                Block::Table(t) => {
                    validate_table(t)?;
                    for c in &t.cells {
                        for p in &c.paragraphs {
                            collect(p);
                        }
                    }
                }
            }
        }
        if result.text.len() > 65535
            || result.para.len() > 65535
            || result.fonts.len() > 65535
            || result
                .fonts
                .iter()
                .any(|f| f.encode_utf16().count() > 65535)
        {
            return Err(error("문서의 서식 종류가 너무 많습니다"));
        }
        Ok(result)
    }
    fn doc_info(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut props = Vec::new();
        for _ in 0..7 {
            put16(&mut props, 1);
        }
        for _ in 0..3 {
            put32(&mut props, 0);
        }
        record(&mut out, 0x10, 0, &props);
        let mut ids = Vec::new();
        put32(&mut ids, 0); // BinData
        for _ in 0..7 {
            put32(&mut ids, self.fonts.len() as u32);
        }
        for count in [
            2,
            self.text.len() as u32,
            1,
            0,
            0,
            self.para.len() as u32,
            1,
            0,
            0,
            0,
        ] {
            put32(&mut ids, count);
        }
        record(&mut out, 0x11, 0, &ids);
        for _ in 0..7 {
            for font in &self.fonts {
                let mut face = vec![0];
                sized_text(&mut face, font);
                record(&mut out, 0x13, 1, &face);
            }
        }
        for bordered in [false, true] {
            let mut border = Vec::new();
            put16(&mut border, 0);
            for i in 0..5 {
                border.push(u8::from(bordered && i < 4));
                border.push(1);
                put32(&mut border, 0);
            }
            put32(&mut border, 0); // no fill
            put32(&mut border, 0); // no extra fill data
            record(&mut out, 0x14, 1, &border);
        }
        for style in &self.text {
            let mut data = Vec::new();
            let font_id = self.fonts.iter().position(|f| f == &style.font).unwrap() as u16;
            for _ in 0..7 {
                put16(&mut data, font_id);
            }
            data.extend_from_slice(&[100; 7]);
            data.extend_from_slice(&[0; 7]);
            data.extend_from_slice(&[100; 7]);
            data.extend_from_slice(&[0; 7]);
            put32(&mut data, style.size * 50);
            put32(
                &mut data,
                u32::from(style.italic)
                    | (u32::from(style.bold) << 1)
                    | (u32::from(style.underline) << 2)
                    | (u32::from(style.strike) << 18),
            );
            data.extend_from_slice(&[0, 0]);
            for color in [style.color, style.color, 0xffffff, 0, style.color] {
                if data.len() == 68 {
                    put16(&mut data, 0);
                }
                put32(&mut data, rgb_to_colorref(color));
            }
            record(&mut out, 0x15, 1, &data);
        }
        record(&mut out, 0x16, 1, &[0; 8]);
        for style in &self.para {
            let mut data = Vec::new();
            put32(
                &mut data,
                ((style.align as u32) << 2) | (u32::from(style.page_break) << 19),
            );
            for value in [
                style.left * 5,
                style.right * 5,
                style.indent * 5,
                style.before as i32 * 5,
                style.after as i32 * 5,
                160,
            ] {
                put32(&mut data, value as u32);
            }
            data.extend_from_slice(&[0; 14]);
            put32(&mut data, 0);
            put32(&mut data, 0);
            put32(&mut data, 160);
            record(&mut out, 0x19, 1, &data);
        }
        let mut style = Vec::new();
        sized_text(&mut style, "바탕글");
        sized_text(&mut style, "Normal");
        style.extend_from_slice(&[0, 0]);
        put16(&mut style, 1042);
        put16(&mut style, 0);
        put16(&mut style, 0);
        put16(&mut style, 0);
        record(&mut out, 0x1a, 1, &style);
        record(&mut out, 0x1e, 0, &[0; 4]);
        record(&mut out, 0x1f, 1, &[0; 20]);
        out
    }
}

fn extended_control(text: &mut Vec<u8>, code: u16, id: &[u8; 4]) {
    put16(text, code);
    text.extend_from_slice(id);
    text.extend_from_slice(&[0; 8]);
    put16(text, code);
}

fn write_paragraph(
    out: &mut Vec<u8>,
    paragraph: &Paragraph,
    styles: &Styles,
    level: u16,
    last: bool,
    section: Option<&Page>,
    table: bool,
) {
    let mut text = Vec::new();
    if section.is_some() {
        extended_control(&mut text, 2, b"dces");
        extended_control(&mut text, 2, b"dloc");
    }
    if table {
        extended_control(&mut text, 11, b" lbt");
    }
    let mut shapes = Vec::new();
    put32(&mut shapes, 0);
    put32(
        &mut shapes,
        paragraph
            .runs
            .first()
            .map(|r| styles.text.iter().position(|s| s == &r.style).unwrap() as u32)
            .unwrap_or(0),
    );
    for (i, run) in paragraph.runs.iter().enumerate() {
        if i > 0 {
            put32(&mut shapes, (text.len() / 2) as u32);
            put32(
                &mut shapes,
                styles.text.iter().position(|s| s == &run.style).unwrap() as u32,
            );
        }
        for ch in run.text.chars() {
            match ch {
                '\t' => {
                    put16(&mut text, 9);
                    put32(&mut text, 0);
                    text.extend_from_slice(&[0; 8]);
                    put16(&mut text, 9);
                }
                '\n' => put16(&mut text, 10),
                _ => put_text(&mut text, &ch.to_string()),
            }
        }
    }
    put16(&mut text, 13);
    let mut header = Vec::new();
    put32(
        &mut header,
        (text.len() / 2) as u32 | if last { 0x80000000 } else { 0 },
    );
    put32(
        &mut header,
        (if table { 1 << 11 } else { 0 }) | (if section.is_some() { 1 << 2 } else { 0 }),
    );
    put16(
        &mut header,
        styles
            .para
            .iter()
            .position(|s| s == &paragraph.style)
            .unwrap_or(0) as u16,
    );
    header.push(0);
    header.push(u8::from(paragraph.style.page_break) * 4);
    put16(&mut header, (shapes.len() / 8) as u16);
    put16(&mut header, 0);
    put16(&mut header, 0); // No stale editor-specific line layout cache.
    put32(&mut header, 0);
    put16(&mut header, 0);
    record(out, PARA_HEADER, level, &header);
    record(out, PARA_TEXT, level + 1, &text);
    record(out, CHAR_POS, level + 1, &shapes);
    if let Some(page) = section {
        write_section(out, page, level + 1);
    }
}

fn write_section(out: &mut Vec<u8>, page: &Page, level: u16) {
    let mut secd = b"dces".to_vec();
    put32(&mut secd, 0);
    put16(&mut secd, 1134);
    put16(&mut secd, 0);
    put16(&mut secd, 0);
    put32(&mut secd, 8000);
    for value in [0, 1, 0, 0, 0, 0] {
        put16(&mut secd, value);
    }
    put32(&mut secd, 0);
    put32(&mut secd, 0);
    record(out, CTRL, level, &secd);
    let mut def = Vec::new();
    put32(&mut def, page.width * 5);
    put32(&mut def, page.height * 5);
    for margin in page.margins {
        put32(&mut def, margin * 5);
    }
    put32(&mut def, 0);
    put32(&mut def, 0);
    record(out, 0x49, level + 1, &def);
    for _ in 0..2 {
        let mut footnote = vec![0; 8];
        put16(&mut footnote, 41);
        put16(&mut footnote, 1);
        put32(&mut footnote, 0xffffffff);
        for value in [850, 567, 283] {
            put16(&mut footnote, value);
        }
        footnote.extend_from_slice(&[1, 1]);
        put32(&mut footnote, 0);
        record(out, 0x4a, level + 1, &footnote);
    }
    for _ in 0..3 {
        let mut fill = Vec::new();
        put32(&mut fill, 1);
        for _ in 0..4 {
            put16(&mut fill, 1417);
        }
        put16(&mut fill, 1);
        record(out, 0x4b, level + 1, &fill);
    }
    let mut cold = b"dloc".to_vec();
    put16(&mut cold, 0x1004);
    put16(&mut cold, 0);
    cold.extend_from_slice(&[0; 8]);
    record(out, CTRL, level, &cold);
}

fn write_table(out: &mut Vec<u8>, table: &Table, styles: &Styles, page: &Page) {
    let width = (page.width - page.margins[0] - page.margins[1]) * 5;
    let mut ctrl = b" lbt".to_vec();
    put32(&mut ctrl, 1 | (4 << 15) | (2 << 18) | (2 << 26));
    put32(&mut ctrl, 0);
    put32(&mut ctrl, 0);
    put32(&mut ctrl, width);
    put32(&mut ctrl, table.rows as u32 * 2000);
    put32(&mut ctrl, 0);
    ctrl.extend_from_slice(&[0; 8]);
    put32(&mut ctrl, 1);
    put32(&mut ctrl, 0);
    put16(&mut ctrl, 0);
    record(out, CTRL, 1, &ctrl);
    let mut props = Vec::new();
    put32(&mut props, 1);
    put16(&mut props, table.rows as u16);
    put16(&mut props, table.cols as u16);
    put16(&mut props, 0);
    for _ in 0..4 {
        put16(&mut props, 141);
    }
    for row in 0..table.rows {
        put16(
            &mut props,
            table.cells.iter().filter(|c| c.row == row).count() as u16,
        );
    }
    put16(&mut props, 2);
    put16(&mut props, 0);
    record(out, 0x4d, 2, &props);
    let mut cells: Vec<_> = table.cells.iter().collect();
    cells.sort_by_key(|c| (c.row, c.col));
    for cell in cells {
        let mut list = Vec::new();
        put32(&mut list, cell.paragraphs.len() as u32);
        put32(&mut list, 0);
        for value in [cell.col, cell.row, cell.col_span, cell.row_span] {
            put16(&mut list, value as u16);
        }
        put32(&mut list, cell.width * 5);
        put32(&mut list, 2000 * cell.row_span as u32);
        for _ in 0..4 {
            put16(&mut list, 141);
        }
        put16(&mut list, 2);
        put32(&mut list, cell.width * 5);
        list.extend_from_slice(&[0; 9]);
        record(out, LIST, 2, &list);
        for (i, paragraph) in cell.paragraphs.iter().enumerate() {
            write_paragraph(
                out,
                paragraph,
                styles,
                2,
                i + 1 == cell.paragraphs.len(),
                None,
                false,
            );
        }
    }
}

pub(super) fn write(document: &OfficeDocument) -> Result<Vec<u8>, Error> {
    validate_document(document)?;
    let styles = Styles::collect(document)?;
    let mut body = Vec::new();
    for (i, block) in document.blocks.iter().enumerate() {
        let section = if i == 0 { Some(&document.page) } else { None };
        let last = i + 1 == document.blocks.len();
        match block {
            Block::Paragraph(p) => write_paragraph(&mut body, p, &styles, 0, last, section, false),
            Block::Table(t) => {
                write_paragraph(
                    &mut body,
                    &Paragraph::default(),
                    &styles,
                    0,
                    last,
                    section,
                    true,
                );
                write_table(&mut body, t, &styles, &document.page);
            }
        }
    }
    let mut file = CompoundFile::create_with_version(cfb::Version::V3, Cursor::new(Vec::new()))
        .map_err(|e| error(e.to_string()))?;
    file.create_storage("/BodyText")?;
    let mut header = vec![0; 256];
    header[..17].copy_from_slice(b"HWP Document File");
    header[32..36].copy_from_slice(&[4, 3, 0, 5]);
    header[36] = 1;
    file.create_stream("/FileHeader")?.write_all(&header)?;
    for (name, data) in [
        ("/DocInfo", styles.doc_info()),
        ("/BodyText/Section0", body),
    ] {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&data)?;
        file.create_stream(name)?.write_all(&encoder.finish()?)?;
    }
    let mut preview = Vec::new();
    for block in &document.blocks {
        if let Block::Paragraph(p) = block {
            for run in &p.runs {
                put_text(&mut preview, &run.text);
            }
            put16(&mut preview, 13);
            put16(&mut preview, 10);
        }
        if preview.len() >= 2048 {
            break;
        }
    }
    file.create_stream("/PrvText")?.write_all(&preview)?;
    file.create_stream("/PrvImage")?;
    file.flush()?;
    Ok(file.into_inner().into_inner())
}
