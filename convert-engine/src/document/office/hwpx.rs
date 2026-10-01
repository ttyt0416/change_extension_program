//! Native OWPML package reader/writer. Format reference: Hancom's public model
//! https://github.com/hancom-io/hwpx-owpml-model and the HWP/OWPML specification.
//! The attribution is exposed through super::HWP_NOTICE in the application's settings.
use super::*;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::Write;

const HP: &str = "http://www.hancom.co.kr/hwpml/2011/paragraph";
const HH: &str = "http://www.hancom.co.kr/hwpml/2011/head";
const HS: &str = "http://www.hancom.co.kr/hwpml/2011/section";
const HC: &str = "http://www.hancom.co.kr/hwpml/2011/core";
const OPF: &str = "http://www.idpf.org/2007/opf/";
const OCF: &str = "urn:oasis:names:tc:opendocument:xmlns:container";
const NS: &str = "xmlns:hp=\"http://www.hancom.co.kr/hwpml/2011/paragraph\" xmlns:hh=\"http://www.hancom.co.kr/hwpml/2011/head\" xmlns:hs=\"http://www.hancom.co.kr/hwpml/2011/section\" xmlns:hc=\"http://www.hancom.co.kr/hwpml/2011/core\"";
const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";

fn elements<'a, 'i>(node: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(|n| n.is_element())
}
fn get<'a, 'i>(node: Node<'a, 'i>, ns: &str, name: &str) -> Result<Node<'a, 'i>, Error> {
    elements(node)
        .find(|n| n.has_tag_name((ns, name)))
        .ok_or_else(|| error(format!("HWPX 필수 요소가 없습니다: {name}")))
}
fn num(node: Node<'_, '_>, name: &str) -> Result<u32, Error> {
    node.attribute(name)
        .and_then(|s| s.parse().ok())
        .filter(|n| *n <= 10_000_000)
        .ok_or_else(|| error(format!("HWPX 숫자 속성이 올바르지 않습니다: {name}")))
}
fn flag(node: Node<'_, '_>, name: &str) -> bool {
    matches!(node.attribute(name), Some("1" | "true"))
}
fn xml(source: &str) -> Result<Document<'_>, Error> {
    Document::parse(source).map_err(|e| error(format!("HWPX XML 오류: {e}")))
}
fn read_entry(archive: &mut zip::ZipArchive<File>, name: &str) -> Result<String, Error> {
    let file = archive
        .by_name(name)
        .map_err(|e| error(format!("HWPX {name}: {e}")))?;
    String::from_utf8(limited_read(file)?).map_err(|e| error(e.to_string()))
}
fn safe_path(path: &str) -> Result<&str, Error> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', ':'])
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(error("HWPX 패키지 경로가 올바르지 않습니다"));
    }
    Ok(path)
}
fn reference_path(package: &str, href: &str, names: &HashSet<String>) -> Result<String, Error> {
    safe_path(href)?;
    let relative = format!(
        "{}{href}",
        package
            .rsplit_once('/')
            .map_or("", |(p, _)| &package[..p.len() + 1])
    );
    if names.contains(&relative) {
        Ok(relative)
    } else if names.contains(href) {
        Ok(href.into())
    } else {
        Err(error(format!("HWPX 참조 파일이 없습니다: {href}")))
    }
}

struct Styles<'a, 'i> {
    chars: HashMap<u32, Node<'a, 'i>>,
    paras: HashMap<u32, Node<'a, 'i>>,
    fonts: HashMap<u32, String>,
}
impl<'a, 'i> Styles<'a, 'i> {
    fn read(header: &'a Document<'i>) -> Result<Self, Error> {
        if !header.root_element().has_tag_name((HH, "head")) {
            return Err(error("HWPX 문서 헤더가 올바르지 않습니다"));
        }
        let refs = get(header.root_element(), HH, "refList")?;
        let mut chars = HashMap::new();
        let mut paras = HashMap::new();
        for (name, item, map) in [
            ("charProperties", "charPr", &mut chars),
            ("paraProperties", "paraPr", &mut paras),
        ] {
            for node in elements(get(refs, HH, name)?) {
                if !node.has_tag_name((HH, item)) || map.insert(num(node, "id")?, node).is_some() {
                    return Err(error("HWPX 서식 ID가 중복되거나 올바르지 않습니다"));
                }
            }
        }
        let face = elements(get(refs, HH, "fontfaces")?)
            .find(|n| n.has_tag_name((HH, "fontface")) && n.attribute("lang") == Some("HANGUL"))
            .ok_or_else(|| error("HWPX 한글 글꼴 정보가 없습니다"))?;
        let mut fonts = HashMap::new();
        for node in elements(face).filter(|n| n.has_tag_name((HH, "font"))) {
            let name = node
                .attribute("face")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| error("HWPX 글꼴 이름이 없습니다"))?;
            if fonts.insert(num(node, "id")?, name.into()).is_some() {
                return Err(error("HWPX 글꼴 ID가 중복됩니다"));
            }
        }
        Ok(Self {
            chars,
            paras,
            fonts,
        })
    }
    fn text(&self, id: u32) -> Result<TextStyle, Error> {
        let n = *self
            .chars
            .get(&id)
            .ok_or_else(|| error("HWPX 글자 서식 참조가 올바르지 않습니다"))?;
        let font = num(get(n, HH, "fontRef")?, "hangul")?;
        let size = num(n, "height")? / 50;
        if size == 0 || size > 8192 {
            return Err(error("HWPX 글자 크기가 지원 범위를 벗어났습니다"));
        }
        let color = n
            .attribute("textColor")
            .and_then(|c| c.strip_prefix('#'))
            .and_then(|c| u32::from_str_radix(c, 16).ok())
            .filter(|c| *c <= 0xffffff)
            .ok_or_else(|| error("HWPX 글자 색상이 올바르지 않습니다"))?;
        Ok(TextStyle {
            font: self
                .fonts
                .get(&font)
                .ok_or_else(|| error("HWPX 글꼴 참조가 올바르지 않습니다"))?
                .clone(),
            size,
            color,
            bold: elements(n).any(|n| n.has_tag_name((HH, "bold"))),
            italic: elements(n).any(|n| n.has_tag_name((HH, "italic"))),
            underline: elements(n).any(|n| {
                n.has_tag_name((HH, "underline"))
                    && n.attribute("type").is_some_and(|t| t != "NONE")
            }),
            strike: elements(n).any(|n| {
                n.has_tag_name((HH, "strikeout"))
                    && n.attribute("shape").is_some_and(|t| t != "NONE")
            }),
        })
    }
    fn paragraph(&self, id: u32) -> Result<ParagraphStyle, Error> {
        let n = *self
            .paras
            .get(&id)
            .ok_or_else(|| error("HWPX 문단 서식 참조가 올바르지 않습니다"))?;
        if elements(n).any(|n| {
            n.has_tag_name((HH, "heading")) && n.attribute("type").is_some_and(|t| t != "NONE")
        }) {
            return Err(error("HWPX 자동 번호·글머리표는 아직 지원하지 않습니다"));
        }
        let align = match get(n, HH, "align")?.attribute("horizontal") {
            Some("JUSTIFY") => 0,
            Some("LEFT") => 1,
            Some("RIGHT") => 2,
            Some("CENTER") => 3,
            Some("DISTRIBUTE" | "DISTRIBUTE_SPACE") => 4,
            _ => return Err(error("HWPX 문단 정렬 값이 올바르지 않습니다")),
        };
        // Prefer the standard HWPUNIT fallback of a compatibility switch.
        let margin = elements(n)
            .find(|n| n.has_tag_name((HH, "margin")))
            .or_else(|| {
                elements(n)
                    .find(|n| n.has_tag_name((HP, "switch")))
                    .and_then(|n| get(n, HP, "default").ok())
                    .and_then(|n| get(n, HH, "margin").ok())
            })
            .ok_or_else(|| error("HWPX 문단 여백 정보가 없습니다"))?;
        let signed = |name| -> Result<i32, Error> {
            let value = get(margin, HC, name)?;
            if value.attribute("unit") != Some("HWPUNIT") {
                return Err(error("지원하지 않는 HWPX 여백 단위입니다"));
            }
            value
                .attribute("value")
                .and_then(|s| s.parse::<i32>().ok())
                .map(|v| v / 5)
                .ok_or_else(|| error("HWPX 문단 여백이 올바르지 않습니다"))
        };
        Ok(ParagraphStyle {
            align,
            left: signed("left")?,
            right: signed("right")?,
            indent: signed("intent")?,
            before: u32::try_from(signed("prev")?)
                .map_err(|_| error("HWPX 문단 간격이 음수입니다"))?,
            after: u32::try_from(signed("next")?)
                .map_err(|_| error("HWPX 문단 간격이 음수입니다"))?,
            page_break: elements(n)
                .any(|n| n.has_tag_name((HH, "breakSetting")) && flag(n, "pageBreakBefore")),
        })
    }
}

fn text(node: Node<'_, '_>) -> Result<String, Error> {
    let mut result = String::new();
    for n in node.children() {
        if n.is_text() {
            result.push_str(n.text().unwrap_or(""));
        } else if n.has_tag_name((HP, "tab")) {
            result.push('\t');
        } else if n.has_tag_name((HP, "lineBreak")) {
            result.push('\n');
        } else if n.has_tag_name((HP, "nbSpace")) {
            result.push('\u{a0}');
        } else if n.has_tag_name((HP, "hyphen")) || n.has_tag_name((HP, "hypen")) {
            result.push('\u{ad}');
        } else if n.is_element() {
            return Err(error(format!(
                "지원하지 않는 HWPX 텍스트 요소입니다: {}",
                n.tag_name().name()
            )));
        }
    }
    Ok(result)
}
fn paragraph(node: Node<'_, '_>, styles: &Styles<'_, '_>) -> Result<Block, Error> {
    if flag(node, "columnBreak") || flag(node, "merged") {
        return Err(error("HWPX 단 나눔·병합 문단은 아직 지원하지 않습니다"));
    }
    let mut p = Paragraph {
        runs: Vec::new(),
        style: styles.paragraph(num(node, "paraPrIDRef")?)?,
    };
    p.style.page_break |= flag(node, "pageBreak");
    let mut tables = Vec::new();
    for n in elements(node) {
        if n.has_tag_name((HP, "linesegarray")) {
            continue;
        } // Editor recomputes layout.
        if !n.has_tag_name((HP, "run")) {
            return Err(error("지원하지 않는 HWPX 문단 요소입니다"));
        }
        for item in elements(n) {
            if item.has_tag_name((HP, "t")) {
                let value = text(item)?;
                if !value.is_empty() {
                    p.runs.push(TextRun {
                        text: value,
                        style: styles.text(num(n, "charPrIDRef")?)?,
                    });
                }
            } else if item.has_tag_name((HP, "tbl")) {
                tables.push(table(item, styles)?);
            } else if item.has_tag_name((HP, "secPr")) { /* processed at section level */
            } else if item.has_tag_name((HP, "ctrl")) {
                for control in elements(item) {
                    if !control.has_tag_name((HP, "colPr")) || num(control, "colCount")? != 1 {
                        return Err(error(
                            "HWPX 다단·각주·필드·머리말/꼬리말 등 복합 컨트롤은 아직 지원하지 않습니다",
                        ));
                    }
                }
            } else {
                return Err(error(format!(
                    "지원하지 않는 HWPX 본문 요소입니다: {}",
                    item.tag_name().name()
                )));
            }
        }
    }
    if tables.is_empty() {
        Ok(Block::Paragraph(p))
    } else if tables.len() == 1 && p.runs.is_empty() && !p.style.page_break {
        Ok(Block::Table(tables.remove(0)))
    } else {
        Err(error(
            "HWPX 표와 본문이 섞인 문단 또는 표 앞 쪽 나눔은 아직 지원하지 않습니다",
        ))
    }
}
fn table(node: Node<'_, '_>, styles: &Styles<'_, '_>) -> Result<Table, Error> {
    let mut table = Table {
        rows: num(node, "rowCnt")? as usize,
        cols: num(node, "colCnt")? as usize,
        cells: Vec::new(),
    };
    if table.rows == 0
        || table.cols == 0
        || table.rows.checked_mul(table.cols).is_none_or(|n| n > 10000)
    {
        return Err(error("지원하지 않는 HWPX 표 크기입니다"));
    }
    for row in elements(node) {
        if row.has_tag_name((HP, "caption")) {
            return Err(error("HWPX 표 캡션은 아직 지원하지 않습니다"));
        }
        if !row.has_tag_name((HP, "tr")) {
            if !matches!(
                row.tag_name().name(),
                "sz" | "pos" | "outMargin" | "inMargin" | "cellzoneList"
            ) || row.tag_name().namespace() != Some(HP)
            {
                return Err(error("지원하지 않는 HWPX 표 요소입니다"));
            }
            continue;
        }
        for tc in elements(row) {
            if !tc.has_tag_name((HP, "tc")) {
                return Err(error("HWPX 표 셀 정보가 올바르지 않습니다"));
            }
            if elements(tc).any(|n| {
                n.tag_name().namespace() != Some(HP)
                    || !matches!(
                        n.tag_name().name(),
                        "subList" | "cellAddr" | "cellSpan" | "cellSz" | "cellMargin"
                    )
            }) {
                return Err(error("지원하지 않는 HWPX 표 셀 요소입니다"));
            }
            let addr = get(tc, HP, "cellAddr")?;
            let span = get(tc, HP, "cellSpan")?;
            let list = get(tc, HP, "subList")?;
            if list.attribute("textDirection") != Some("HORIZONTAL") {
                return Err(error("HWPX 세로쓰기 표는 아직 지원하지 않습니다"));
            }
            let mut paragraphs = Vec::new();
            for p in elements(list) {
                if !p.has_tag_name((HP, "p")) {
                    return Err(error("지원하지 않는 HWPX 셀 요소입니다"));
                }
                let Block::Paragraph(p) = paragraph(p, styles)? else {
                    return Err(error("HWPX 중첩 표는 아직 지원하지 않습니다"));
                };
                paragraphs.push(p);
            }
            table.cells.push(Cell {
                row: num(addr, "rowAddr")? as usize,
                col: num(addr, "colAddr")? as usize,
                row_span: num(span, "rowSpan")? as usize,
                col_span: num(span, "colSpan")? as usize,
                width: num(get(tc, HP, "cellSz")?, "width")? / 5,
                paragraphs,
            });
        }
    }
    validate_table(&table)?;
    Ok(table)
}
fn page(section: Node<'_, '_>) -> Result<Page, Error> {
    let properties: Vec<_> = section
        .descendants()
        .filter(|n| n.has_tag_name((HP, "secPr")))
        .collect();
    if properties.len() != 1 {
        return Err(error("HWPX 구역 설정이 없거나 중복됩니다"));
    }
    let props = properties[0];
    if elements(props).any(|n| {
        n.tag_name().namespace() != Some(HP)
            || !matches!(
                n.tag_name().name(),
                "grid"
                    | "startNum"
                    | "visibility"
                    | "lineNumberShape"
                    | "pagePr"
                    | "footNotePr"
                    | "endNotePr"
                    | "pageBorderFill"
            )
    }) {
        return Err(error("지원하지 않는 HWPX 구역 설정 요소입니다"));
    }
    if props.attribute("textDirection") != Some("HORIZONTAL") || num(props, "masterPageCnt")? != 0 {
        return Err(error("HWPX 세로쓰기·바탕쪽은 아직 지원하지 않습니다"));
    }
    let size = get(props, HP, "pagePr")?;
    let margin = get(size, HP, "margin")?;
    if num(margin, "gutter")? != 0 {
        return Err(error("HWPX 제본 여백은 아직 지원하지 않습니다"));
    }
    let mut result = Page {
        width: num(size, "width")? / 5,
        height: num(size, "height")? / 5,
        margins: [0; 6],
    };
    for (i, name) in ["left", "right", "top", "bottom", "header", "footer"]
        .iter()
        .enumerate()
    {
        result.margins[i] = num(margin, name)? / 5;
    }
    validate_page(&result)?;
    Ok(result)
}

pub(super) fn read(path: &Path) -> Result<OfficeDocument, Error> {
    let mut archive = zip::ZipArchive::new(File::open(path)?).map_err(|e| error(e.to_string()))?;
    if archive.len() > 10000 {
        return Err(error("HWPX 패키지 파일 수가 너무 많습니다"));
    }
    let mut names = HashSet::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| error(e.to_string()))?;
        if !names.insert(entry.name().to_string()) {
            return Err(error("HWPX 패키지에 중복 파일이 있습니다"));
        }
        if entry.name().contains("encrypt")
            || entry.name().contains("distribute")
            || entry.name().contains("Drm")
        {
            return Err(error("암호·배포 제한·DRM HWPX는 지원하지 않습니다"));
        }
    }
    if read_entry(&mut archive, "mimetype")?.trim() != "application/hwp+zip" {
        return Err(error("HWPX 파일이 아닙니다"));
    }
    if names.contains("META-INF/manifest.xml") {
        let source = read_entry(&mut archive, "META-INF/manifest.xml")?;
        if xml(&source)?
            .descendants()
            .any(|n| n.is_element() && n.tag_name().name() == "encryption-data")
        {
            return Err(error("암호화된 HWPX는 지원하지 않습니다"));
        }
    }
    let container = read_entry(&mut archive, "META-INF/container.xml")?;
    let container = xml(&container)?;
    let package = container
        .descendants()
        .find(|n| {
            n.has_tag_name((OCF, "rootfile"))
                && n.attribute("media-type") == Some("application/hwpml-package+xml")
        })
        .and_then(|n| n.attribute("full-path"))
        .ok_or_else(|| error("HWPX 패키지 위치가 없습니다"))?;
    safe_path(package)?;
    let content = read_entry(&mut archive, package)?;
    let content = xml(&content)?;
    let root = content.root_element();
    if !root.has_tag_name((OPF, "package")) {
        return Err(error("HWPX 패키지 정보가 올바르지 않습니다"));
    }
    let mut items = HashMap::new();
    for node in elements(get(root, OPF, "manifest")?) {
        let id = node
            .attribute("id")
            .ok_or_else(|| error("HWPX 패키지 항목 ID가 없습니다"))?;
        let path = reference_path(
            package,
            node.attribute("href")
                .ok_or_else(|| error("HWPX 패키지 항목 경로가 없습니다"))?,
            &names,
        )?;
        if items.insert(id, path).is_some() {
            return Err(error("HWPX 패키지 항목 ID가 중복됩니다"));
        }
    }
    let header_path = items
        .get("header")
        .ok_or_else(|| error("HWPX 헤더 참조가 없습니다"))?;
    let header_source = read_entry(&mut archive, header_path)?;
    let header = xml(&header_source)?;
    let styles = Styles::read(&header)?;
    let mut result = OfficeDocument::default();
    let mut count = 0;
    let mut used = HashSet::new();
    let mut total = header_source.len() as u64;
    for item in elements(get(root, OPF, "spine")?) {
        let id = item
            .attribute("idref")
            .ok_or_else(|| error("HWPX 읽기 순서 참조가 없습니다"))?;
        if !used.insert(id) {
            return Err(error("HWPX 읽기 순서 참조가 중복됩니다"));
        }
        if id == "header" {
            continue;
        }
        let path = items
            .get(id)
            .ok_or_else(|| error("HWPX 구역 참조가 올바르지 않습니다"))?;
        let source = read_entry(&mut archive, path)?;
        total += source.len() as u64;
        if total > MAX_STREAM * 2 {
            return Err(error("HWPX 전체 본문 데이터가 너무 큽니다"));
        }
        let section = xml(&source)?;
        let root = section.root_element();
        if !root.has_tag_name((HS, "sec")) {
            return Err(error("지원하지 않는 HWPX 읽기 순서 항목입니다"));
        }
        let page = page(root)?;
        if count == 0 {
            result.page = page;
        } else if result.page != page {
            return Err(error("용지 설정이 다른 HWPX 구역은 아직 지원하지 않습니다"));
        }
        let first = result.blocks.len();
        for p in elements(root) {
            if !p.has_tag_name((HP, "p")) {
                return Err(error("지원하지 않는 HWPX 구역 요소입니다"));
            }
            result.blocks.push(paragraph(p, &styles)?);
        }
        if count > 0 {
            if let Some(Block::Paragraph(p)) = result.blocks.get_mut(first) {
                p.style.page_break = true;
            } else {
                result.blocks.insert(
                    first,
                    Block::Paragraph(Paragraph {
                        style: ParagraphStyle {
                            page_break: true,
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                );
            }
        }
        count += 1;
    }
    if count == 0 || result.blocks.is_empty() {
        return Err(error("HWPX 본문 구역이 없습니다"));
    }
    if num(header.root_element(), "secCnt")? as usize != count {
        return Err(error("HWPX 구역 수와 읽기 순서가 일치하지 않습니다"));
    }
    validate_document(&result)?;
    Ok(result)
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}
#[derive(Default)]
struct Writer {
    fonts: Vec<String>,
    chars: Vec<TextStyle>,
    paras: Vec<ParagraphStyle>,
    id: u32,
}
fn index<T: PartialEq + Clone>(items: &mut Vec<T>, value: &T) -> usize {
    if let Some(i) = items.iter().position(|item| item == value) {
        i
    } else {
        items.push(value.clone());
        items.len() - 1
    }
}
impl Writer {
    fn register(&mut self, p: &Paragraph) {
        index(&mut self.paras, &p.style);
        for r in &p.runs {
            index(&mut self.chars, &r.style);
            index(&mut self.fonts, &r.style.font);
        }
    }
    fn header(&self) -> String {
        let mut out = format!(
            "{XML}<hh:head {NS} version=\"1.4\" secCnt=\"1\"><hh:beginNum page=\"1\" footnote=\"1\" endnote=\"1\" pic=\"1\" tbl=\"1\" equation=\"1\"/><hh:refList><hh:fontfaces itemCnt=\"7\">"
        );
        for lang in [
            "HANGUL", "LATIN", "HANJA", "JAPANESE", "OTHER", "SYMBOL", "USER",
        ] {
            write!(
                out,
                "<hh:fontface lang=\"{lang}\" fontCnt=\"{}\">",
                self.fonts.len()
            )
            .unwrap();
            for (id, font) in self.fonts.iter().enumerate() {
                write!(
                    out,
                    "<hh:font id=\"{id}\" face=\"{}\" type=\"TTF\" isEmbedded=\"0\"/>",
                    escape(font)
                )
                .unwrap();
            }
            out.push_str("</hh:fontface>");
        }
        out.push_str("</hh:fontfaces><hh:borderFills itemCnt=\"2\">");
        for id in 1..=2 {
            write!(out, "<hh:borderFill id=\"{id}\" threeD=\"0\" shadow=\"0\" centerLine=\"NONE\" breakCellSeparateLine=\"0\"><hh:slash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/><hh:backSlash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/>").unwrap();
            for side in [
                "leftBorder",
                "rightBorder",
                "topBorder",
                "bottomBorder",
                "diagonal",
            ] {
                write!(
                    out,
                    "<hh:{side} type=\"{}\" width=\"0.1 mm\" color=\"#000000\"/>",
                    if id == 2 { "SOLID" } else { "NONE" }
                )
                .unwrap();
            }
            out.push_str("</hh:borderFill>");
        }
        write!(
            out,
            "</hh:borderFills><hh:charProperties itemCnt=\"{}\">",
            self.chars.len()
        )
        .unwrap();
        for (id, s) in self.chars.iter().enumerate() {
            write!(out, "<hh:charPr id=\"{id}\" height=\"{}\" textColor=\"#{:06X}\" shadeColor=\"none\" useFontSpace=\"0\" useKerning=\"0\" symMark=\"NONE\" borderFillIDRef=\"1\">", s.size*50, s.color).unwrap();
            for (tag, v) in [
                (
                    "fontRef",
                    self.fonts.iter().position(|f| f == &s.font).unwrap() as i32,
                ),
                ("ratio", 100),
                ("spacing", 0),
                ("relSz", 100),
                ("offset", 0),
            ] {
                write!(out, "<hh:{tag} hangul=\"{v}\" latin=\"{v}\" hanja=\"{v}\" japanese=\"{v}\" other=\"{v}\" symbol=\"{v}\" user=\"{v}\"/>").unwrap();
            }
            if s.bold {
                out.push_str("<hh:bold/>");
            }
            if s.italic {
                out.push_str("<hh:italic/>");
            }
            write!(out, "<hh:underline type=\"{}\" shape=\"SOLID\" color=\"#{:06X}\"/><hh:strikeout shape=\"{}\" color=\"#{:06X}\"/><hh:outline type=\"NONE\"/><hh:shadow type=\"NONE\" color=\"#B2B2B2\" offsetX=\"0\" offsetY=\"0\"/></hh:charPr>", if s.underline {"BOTTOM"} else {"NONE"}, s.color, if s.strike {"SOLID"} else {"NONE"}, s.color).unwrap();
        }
        write!(out, "</hh:charProperties><hh:tabProperties itemCnt=\"1\"><hh:tabPr id=\"0\" autoTabLeft=\"0\" autoTabRight=\"0\"/></hh:tabProperties><hh:paraProperties itemCnt=\"{}\">", self.paras.len()).unwrap();
        for (id, s) in self.paras.iter().enumerate() {
            let align = match s.align {
                0 => "JUSTIFY",
                2 => "RIGHT",
                3 => "CENTER",
                4 | 5 => "DISTRIBUTE",
                _ => "LEFT",
            };
            write!(out, "<hh:paraPr id=\"{id}\" tabPrIDRef=\"0\" condense=\"0\" fontLineHeight=\"0\" snapToGrid=\"1\" suppressLineNumbers=\"0\" checked=\"0\"><hh:align horizontal=\"{align}\" vertical=\"BASELINE\"/><hh:heading type=\"NONE\" idRef=\"0\" level=\"0\"/><hh:breakSetting breakLatinWord=\"KEEP_WORD\" breakNonLatinWord=\"BREAK_WORD\" widowOrphan=\"0\" keepWithNext=\"0\" keepLines=\"0\" pageBreakBefore=\"{}\" lineWrap=\"BREAK\"/><hh:margin>", u8::from(s.page_break)).unwrap();
            for (tag, v) in [
                ("intent", s.indent),
                ("left", s.left),
                ("right", s.right),
                ("prev", s.before as i32),
                ("next", s.after as i32),
            ] {
                write!(out, "<hc:{tag} value=\"{}\" unit=\"HWPUNIT\"/>", v * 5).unwrap();
            }
            out.push_str("</hh:margin><hh:lineSpacing type=\"PERCENT\" value=\"160\" unit=\"HWPUNIT\"/><hh:autoSpacing eAsianEng=\"0\" eAsianNum=\"0\"/><hh:border borderFillIDRef=\"1\" offsetLeft=\"0\" offsetRight=\"0\" offsetTop=\"0\" offsetBottom=\"0\" connect=\"0\" ignoreMargin=\"0\"/></hh:paraPr>");
        }
        out.push_str("</hh:paraProperties><hh:styles itemCnt=\"1\"><hh:style id=\"0\" type=\"PARA\" name=\"바탕글\" engName=\"Normal\" paraPrIDRef=\"0\" charPrIDRef=\"0\" nextStyleIDRef=\"0\" langID=\"1042\" lockForm=\"0\"/></hh:styles></hh:refList><hh:compatibleDocument targetProgram=\"HWP201X\"><hh:layoutCompatibility/></hh:compatibleDocument><hh:docOption><hh:linkinfo path=\"\" pageInherit=\"0\" footnoteInherit=\"0\"/></hh:docOption></hh:head>");
        out
    }
    fn paragraph(&mut self, p: &Paragraph, extra: &str) -> String {
        self.id += 1;
        let id = self.paras.iter().position(|s| s == &p.style).unwrap();
        let mut out = format!(
            "<hp:p id=\"{}\" paraPrIDRef=\"{id}\" styleIDRef=\"0\" pageBreak=\"{}\" columnBreak=\"0\" merged=\"0\">",
            self.id,
            u8::from(p.style.page_break)
        );
        if !extra.is_empty() {
            write!(out, "<hp:run charPrIDRef=\"0\">{extra}</hp:run>").unwrap();
        }
        for r in &p.runs {
            let id = self.chars.iter().position(|s| s == &r.style).unwrap();
            write!(out, "<hp:run charPrIDRef=\"{id}\"><hp:t>").unwrap();
            for segment in r.text.split_inclusive(['\t', '\n']) {
                let plain = segment.trim_end_matches(['\t', '\n']);
                out.push_str(&escape(plain));
                if segment.ends_with('\t') {
                    out.push_str("<hp:tab width=\"4000\" leader=\"0\" type=\"0\"/>");
                } else if segment.ends_with('\n') {
                    out.push_str("<hp:lineBreak/>");
                }
            }
            out.push_str("</hp:t></hp:run>");
        }
        if p.runs.is_empty() {
            out.push_str("<hp:run charPrIDRef=\"0\"><hp:t/></hp:run>");
        }
        out.push_str("</hp:p>");
        out
    }
    fn table(&mut self, table: &Table) -> Result<String, Error> {
        self.id += 1;
        let width: u32 = table
            .cells
            .iter()
            .filter(|c| c.row == 0)
            .try_fold(0u32, |sum, c| sum.checked_add(c.width * 5))
            .ok_or_else(|| error("HWPX 표의 전체 너비가 너무 큽니다"))?;
        let mut out = format!(
            "<hp:tbl id=\"{}\" zOrder=\"0\" numberingType=\"TABLE\" textWrap=\"TOP_AND_BOTTOM\" textFlow=\"BOTH_SIDES\" lock=\"0\" dropcapstyle=\"None\" pageBreak=\"CELL\" repeatHeader=\"0\" rowCnt=\"{}\" colCnt=\"{}\" cellSpacing=\"0\" borderFillIDRef=\"2\" noAdjust=\"0\"><hp:sz width=\"{width}\" widthRelTo=\"ABSOLUTE\" height=\"{}\" heightRelTo=\"ABSOLUTE\" protect=\"0\"/><hp:pos treatAsChar=\"1\" affectLSpacing=\"0\" flowWithText=\"1\" allowOverlap=\"0\" holdAnchorAndSO=\"0\" vertRelTo=\"PARA\" horzRelTo=\"COLUMN\" vertAlign=\"TOP\" horzAlign=\"LEFT\" vertOffset=\"0\" horzOffset=\"0\"/><hp:outMargin left=\"0\" right=\"0\" top=\"0\" bottom=\"0\"/><hp:inMargin left=\"100\" right=\"100\" top=\"100\" bottom=\"100\"/>",
            self.id,
            table.rows,
            table.cols,
            table.rows * 1500
        );
        for row in 0..table.rows {
            out.push_str("<hp:tr>");
            let mut cells: Vec<_> = table.cells.iter().filter(|c| c.row == row).collect();
            cells.sort_by_key(|c| c.col);
            for c in cells {
                out.push_str("<hp:tc name=\"\" header=\"0\" hasMargin=\"1\" protect=\"0\" editable=\"0\" dirty=\"0\" borderFillIDRef=\"2\"><hp:subList id=\"\" textDirection=\"HORIZONTAL\" lineWrap=\"BREAK\" vertAlign=\"TOP\" linkListIDRef=\"0\" linkListNextIDRef=\"0\" textWidth=\"0\" textHeight=\"0\" hasTextRef=\"0\" hasNumRef=\"0\">");
                for p in &c.paragraphs {
                    out.push_str(&self.paragraph(p, ""));
                }
                write!(out, "</hp:subList><hp:cellAddr colAddr=\"{}\" rowAddr=\"{}\"/><hp:cellSpan colSpan=\"{}\" rowSpan=\"{}\"/><hp:cellSz width=\"{}\" height=\"{}\"/><hp:cellMargin left=\"100\" right=\"100\" top=\"100\" bottom=\"100\"/></hp:tc>", c.col,c.row,c.col_span,c.row_span,c.width*5,c.row_span*1500).unwrap();
            }
            out.push_str("</hp:tr>");
        }
        out.push_str("</hp:tbl>");
        Ok(out)
    }
}
fn section_properties(page: &Page) -> String {
    let m = page.margins.map(|v| v * 5);
    format!(
        "<hp:secPr id=\"\" textDirection=\"HORIZONTAL\" spaceColumns=\"0\" tabStop=\"8000\" tabStopVal=\"4000\" tabStopUnit=\"HWPUNIT\" outlineShapeIDRef=\"0\" memoShapeIDRef=\"0\" textVerticalWidthHead=\"0\" masterPageCnt=\"0\"><hp:grid lineGrid=\"0\" charGrid=\"0\" wonggojiFormat=\"0\"/><hp:startNum pageStartsOn=\"BOTH\" page=\"0\" pic=\"0\" tbl=\"0\" equation=\"0\"/><hp:visibility hideFirstHeader=\"0\" hideFirstFooter=\"0\" hideFirstMasterPage=\"0\" border=\"SHOW_ALL\" fill=\"SHOW_ALL\" hideFirstPageNum=\"0\" hideFirstEmptyLine=\"0\" showLineNumber=\"0\"/><hp:pagePr landscape=\"WIDELY\" width=\"{}\" height=\"{}\" gutterType=\"LEFT_ONLY\"><hp:margin header=\"{}\" footer=\"{}\" gutter=\"0\" left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\"/></hp:pagePr></hp:secPr><hp:ctrl><hp:colPr id=\"\" type=\"NEWSPAPER\" layout=\"LEFT\" colCount=\"1\" sameSz=\"1\" sameGap=\"0\"/></hp:ctrl>",
        page.width * 5,
        page.height * 5,
        m[4],
        m[5],
        m[0],
        m[1],
        m[2],
        m[3]
    )
}
pub(super) fn write(document: &OfficeDocument) -> Result<Vec<u8>, Error> {
    validate_document(document)?;
    let mut writer = Writer {
        fonts: vec![TextStyle::default().font],
        chars: vec![TextStyle::default()],
        paras: vec![ParagraphStyle::default()],
        id: 0,
    };
    for block in &document.blocks {
        match block {
            Block::Paragraph(p) => writer.register(p),
            Block::Table(t) => {
                for c in &t.cells {
                    for p in &c.paragraphs {
                        writer.register(p);
                    }
                }
            }
        }
    }
    let header = writer.header();
    let mut section = format!("{XML}<hs:sec {NS}>");
    let mut extra = section_properties(&document.page);
    for block in &document.blocks {
        match block {
            Block::Paragraph(p) => section.push_str(&writer.paragraph(p, &extra)),
            Block::Table(t) => {
                let table = writer.table(t)?;
                section
                    .push_str(&writer.paragraph(&Paragraph::default(), &format!("{extra}{table}")));
            }
        }
        extra.clear();
    }
    if document.blocks.is_empty() {
        section.push_str(&writer.paragraph(&Paragraph::default(), &extra));
    }
    section.push_str("</hs:sec>");
    // Parse before packaging to reject XML-incompatible characters in source documents.
    xml(&header)?;
    xml(&section)?;
    let content = format!(
        "{XML}<opf:package xmlns:opf=\"{OPF}\" version=\"1.0\" unique-identifier=\"\" id=\"\"><opf:metadata><opf:title/><opf:language>ko</opf:language></opf:metadata><opf:manifest><opf:item id=\"header\" href=\"Contents/header.xml\" media-type=\"application/xml\"/><opf:item id=\"section0\" href=\"Contents/section0.xml\" media-type=\"application/xml\"/><opf:item id=\"settings\" href=\"settings.xml\" media-type=\"application/xml\"/></opf:manifest><opf:spine><opf:itemref idref=\"header\"/><opf:itemref idref=\"section0\"/></opf:spine></opf:package>"
    );
    let container = format!(
        "{XML}<ocf:container xmlns:ocf=\"{OCF}\"><ocf:rootfiles><ocf:rootfile full-path=\"Contents/content.hpf\" media-type=\"application/hwpml-package+xml\"/></ocf:rootfiles></ocf:container>"
    );
    let version = format!(
        "{XML}<hv:HCFVersion xmlns:hv=\"http://www.hancom.co.kr/hwpml/2011/version\" tagetApplication=\"WORDPROCESSOR\" major=\"5\" minor=\"0\" micro=\"5\" buildNumber=\"0\" xmlVersion=\"1.4\" application=\"photo_tools\" appVersion=\"0.1.0\"/>"
    );
    let settings = format!(
        "{XML}<ha:HWPApplicationSetting xmlns:ha=\"http://www.hancom.co.kr/hwpml/2011/app\"><ha:CaretPosition listIDRef=\"0\" paraIDRef=\"0\" pos=\"0\"/></ha:HWPApplicationSetting>"
    );
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in [
        ("mimetype", "application/hwp+zip"),
        ("version.xml", &version),
        ("META-INF/container.xml", &container),
        (
            "META-INF/manifest.xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><odf:manifest xmlns:odf=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"/>",
        ),
        ("Contents/content.hpf", &content),
        ("Contents/header.xml", &header),
        ("Contents/section0.xml", &section),
        ("settings.xml", &settings),
    ] {
        let options = zip::write::SimpleFileOptions::default().compression_method(
            if name == "mimetype" || name == "version.xml" {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            },
        );
        zip.start_file(name, options)
            .map_err(|e| error(e.to_string()))?;
        zip.write_all(data.as_bytes())?;
    }
    Ok(zip.finish().map_err(|e| error(e.to_string()))?.into_inner())
}
