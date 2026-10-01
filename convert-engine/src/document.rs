use crate::{Category, Error, unsupported};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const OUTPUT_FORMATS: &[&str] = &["TXT", "MD", "HTML", "CSV", "TSV", "PDF"];

pub fn output_formats_for(input: &Path) -> &'static [&'static str] {
    let extension = input
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("txt") {
        if fs::read_to_string(input).is_ok() {
            &["MD", "HTML"]
        } else {
            &[]
        }
    } else if extension.eq_ignore_ascii_case("md") {
        match fs::read_to_string(input) {
            Ok(markdown) if html_text(&markdown_html(&markdown)).is_ok() => &["TXT", "HTML"],
            Ok(_) => &["HTML"],
            Err(_) => &[],
        }
    } else if extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm") {
        match fs::read_to_string(input) {
            Ok(html) if html_text(&html).is_ok() => &["TXT"],
            _ => &[],
        }
    } else if extension.eq_ignore_ascii_case("csv") {
        match fs::read(input) {
            Ok(bytes) if delimited(&bytes, b',', b'\t').is_ok() => &["TSV"],
            _ => &[],
        }
    } else if extension.eq_ignore_ascii_case("tsv") {
        match fs::read(input) {
            Ok(bytes) if delimited(&bytes, b'\t', b',').is_ok() => &["CSV"],
            _ => &[],
        }
    } else if extension.eq_ignore_ascii_case("pdf") {
        match pdf_extract::extract_text(input) {
            Ok(text) if !text.trim().is_empty() => &["TXT"],
            _ => &[],
        }
    } else if extension.eq_ignore_ascii_case("docx") {
        let txt = office_text(input, "word/document.xml", OfficeKind::Docx).is_ok();
        let pdf = office2pdf::convert(input).is_ok();
        match (txt, pdf) {
            (true, true) => &["TXT", "PDF"],
            (true, false) => &["TXT"],
            (false, true) => &["PDF"],
            (false, false) => &[],
        }
    } else if extension.eq_ignore_ascii_case("odt") {
        if office_text(input, "content.xml", OfficeKind::Odt).is_ok() {
            &["TXT"]
        } else {
            &[]
        }
    } else {
        &[]
    }
}

pub fn convert(
    input: &Path,
    target_extension: &str,
    output_folder: &Path,
) -> Result<PathBuf, Error> {
    let source = input
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let output = match (source.as_str(), target_extension) {
        ("txt", "md") => fs::read_to_string(input)?.into_bytes(),
        ("txt", "html") => plain_html(&fs::read_to_string(input)?).into_bytes(),
        ("md", "html") => markdown_html(&fs::read_to_string(input)?).into_bytes(),
        ("md", "txt") => html_text(&markdown_html(&fs::read_to_string(input)?))?.into_bytes(),
        ("html" | "htm", "txt") => html_text(&fs::read_to_string(input)?)?.into_bytes(),
        ("csv", "tsv") => delimited(&fs::read(input)?, b',', b'\t')?,
        ("tsv", "csv") => delimited(&fs::read(input)?, b'\t', b',')?,
        ("pdf", "txt") => {
            let text = pdf_extract::extract_text(input)
                .map_err(|error| Error::Document(error.to_string()))?;
            if text.trim().is_empty() {
                return Err(Error::Document(
                    "PDF에서 추출할 수 있는 텍스트가 없습니다".to_owned(),
                ));
            }
            text.into_bytes()
        }
        ("docx", "txt") => office_text(input, "word/document.xml", OfficeKind::Docx)?.into_bytes(),
        ("docx", "pdf") => {
            office2pdf::convert(input)
                .map_err(|error| Error::Document(error.to_string()))?
                .pdf
        }
        ("odt", "txt") => office_text(input, "content.xml", OfficeKind::Odt)?.into_bytes(),
        _ => return Err(unsupported(Category::Document, target_extension)),
    };
    write_output(input, target_extension, output_folder, &output)
}

fn markdown_html(markdown: &str) -> String {
    let mut html = String::from("<!doctype html><html><meta charset=\"utf-8\"><body>");
    pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new(markdown));
    html.push_str("</body></html>");
    html
}

fn html_text(html: &str) -> Result<String, Error> {
    html2text::from_read(html.as_bytes(), 10_000)
        .map_err(|error| Error::Document(error.to_string()))
}

fn plain_html(text: &str) -> String {
    let mut html = String::from("<!doctype html><html><meta charset=\"utf-8\"><body><pre>");
    for character in text.chars() {
        match character {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '"' => html.push_str("&quot;"),
            _ => html.push(character),
        }
    }
    html.push_str("</pre></body></html>");
    html
}

fn delimited(input: &[u8], source: u8, target: u8) -> Result<Vec<u8>, Error> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(source)
        .from_reader(input);
    let mut writer = csv::WriterBuilder::new()
        .has_headers(false)
        .delimiter(target)
        .from_writer(Vec::new());
    for row in reader.byte_records() {
        writer
            .write_byte_record(&row.map_err(|error| Error::Document(error.to_string()))?)
            .map_err(|error| Error::Document(error.to_string()))?;
    }
    writer
        .into_inner()
        .map_err(|error| Error::Document(error.to_string()))
}

enum OfficeKind {
    Docx,
    Odt,
}

fn office_text(input: &Path, entry_name: &str, kind: OfficeKind) -> Result<String, Error> {
    let mut archive = zip::ZipArchive::new(File::open(input)?)
        .map_err(|error| Error::Document(error.to_string()))?;
    let mut xml = String::new();
    archive
        .by_name(entry_name)
        .map_err(|error| Error::Document(error.to_string()))?
        .read_to_string(&mut xml)?;
    let document =
        roxmltree::Document::parse(&xml).map_err(|error| Error::Document(error.to_string()))?;
    let mut text = String::new();
    match kind {
        OfficeKind::Docx => {
            const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
            for node in document.descendants() {
                if node.is_text()
                    && node
                        .parent()
                        .is_some_and(|parent| parent.has_tag_name((W, "t")))
                {
                    text.push_str(node.text().unwrap_or_default());
                } else if node.has_tag_name((W, "tab")) {
                    text.push('\t');
                } else if node.has_tag_name((W, "br")) {
                    text.push('\n');
                } else if node.has_tag_name((W, "p")) && !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
            }
        }
        OfficeKind::Odt => {
            const T: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
            for paragraph in document
                .descendants()
                .filter(|node| node.has_tag_name((T, "p")) || node.has_tag_name((T, "h")))
            {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                for node in paragraph.descendants() {
                    if node.is_text() {
                        text.push_str(node.text().unwrap_or_default());
                    } else if node.has_tag_name((T, "tab")) {
                        text.push('\t');
                    } else if node.has_tag_name((T, "line-break")) {
                        text.push('\n');
                    } else if node.has_tag_name((T, "s")) {
                        let count = node
                            .attribute((T, "c"))
                            .and_then(|count| count.parse::<usize>().ok())
                            .unwrap_or(1);
                        for _ in 0..count.min(10_000) {
                            text.push(' ');
                        }
                    }
                }
            }
        }
    }
    if text.trim().is_empty() {
        return Err(Error::Document(
            "문서에서 추출할 수 있는 텍스트가 없습니다".to_owned(),
        ));
    }
    Ok(text)
}

fn write_output(
    input: &Path,
    extension: &str,
    folder: &Path,
    contents: &[u8],
) -> Result<PathBuf, Error> {
    let stem = input
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| OsStr::new("output"));
    for index in 0..10_000 {
        let mut name = OsString::from(stem);
        if index > 0 {
            name.push(format!("_{index}"));
        }
        name.push(".");
        name.push(extension);
        let path = folder.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(contents).and_then(|()| file.flush()) {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(error.into());
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Document(
        "사용 가능한 출력 파일 이름이 없습니다".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(name: &str) -> PathBuf {
        let unique = format!(
            "convert-engine-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        fs::create_dir(&directory).unwrap();
        directory
    }

    #[test]
    fn text_and_markup_conversions_keep_content_and_avoid_overwriting() {
        let directory = test_directory("markup");
        let input = directory.join("note.txt");
        fs::write(&input, "한글 <태그> & 내용\n").unwrap();
        assert_eq!(output_formats_for(&input), &["MD", "HTML"]);
        let first = convert(&input, "html", &directory).unwrap();
        let second = convert(&input, "html", &directory).unwrap();
        assert_ne!(first, second);
        assert!(
            fs::read_to_string(first)
                .unwrap()
                .contains("한글 &lt;태그&gt; &amp; 내용")
        );
        let markdown = convert(&input, "md", &directory).unwrap();
        assert_eq!(
            fs::read_to_string(markdown).unwrap(),
            "한글 <태그> & 내용\n"
        );
        let markdown_input = directory.join("heading.md");
        fs::write(&markdown_input, "# 제목\n\n**강조**\n").unwrap();
        let html = convert(&markdown_input, "html", &directory).unwrap();
        assert!(fs::read_to_string(html).unwrap().contains("<h1>제목</h1>"));
        let plain = convert(&markdown_input, "txt", &directory).unwrap();
        assert!(fs::read_to_string(plain).unwrap().contains("강조"));
        assert!(matches!(
            convert(&input, "pdf", &directory),
            Err(Error::UnsupportedFormat { .. })
        ));
        let invalid = directory.join("invalid.txt");
        fs::write(&invalid, [0xff, 0xfe]).unwrap();
        assert!(output_formats_for(&invalid).is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn csv_tsv_round_trip_preserves_quoted_fields() {
        let directory = test_directory("table");
        let input = directory.join("data.csv");
        fs::write(&input, "name,value\n\"a,b\",\"line1\nline2\"\n").unwrap();
        let tsv = convert(&input, "tsv", &directory).unwrap();
        let csv = convert(&tsv, "csv", &directory).unwrap();
        assert_eq!(fs::read(csv).unwrap(), fs::read(input).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn extracts_text_from_docx_and_odt() {
        let directory = test_directory("office");
        for (name, entry, xml, expected) in [
            (
                "sample.docx",
                "word/document.xml",
                "<w:document xmlns:w='http://schemas.openxmlformats.org/wordprocessingml/2006/main'><w:body><w:p><w:r><w:t>첫째 &amp; 둘째</w:t></w:r></w:p><w:p><w:r><w:t>다음</w:t></w:r></w:p></w:body></w:document>",
                "첫째 & 둘째\n다음",
            ),
            (
                "sample.odt",
                "content.xml",
                "<office:document-content xmlns:office='urn:oasis:names:tc:opendocument:xmlns:office:1.0' xmlns:text='urn:oasis:names:tc:opendocument:xmlns:text:1.0'><office:body><office:text><text:p>첫째 &amp; 둘째</text:p><text:p>다음</text:p></office:text></office:body></office:document-content>",
                "첫째 & 둘째\n다음",
            ),
        ] {
            let input = directory.join(name);
            let mut archive = zip::ZipWriter::new(File::create(&input).unwrap());
            archive
                .start_file(entry, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(xml.as_bytes()).unwrap();
            archive.finish().unwrap();
            let output = convert(&input, "txt", &directory).unwrap();
            assert_eq!(fs::read_to_string(output).unwrap(), expected);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn extracts_text_from_pdf() {
        use lopdf::content::{Content, Operation};
        use lopdf::{Document, Object, Stream, dictionary};

        let directory = test_directory("pdf");
        let input = directory.join("sample.pdf");
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 24.into()]),
                Operation::new("Td", vec![40.into(), 100.into()]),
                Operation::new("Tj", vec![Object::string_literal("Hello PDF")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id =
            document.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.save(&input).unwrap();

        let output = convert(&input, "txt", &directory).unwrap();
        assert!(fs::read_to_string(output).unwrap().contains("Hello PDF"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn converts_docx_to_pdf() {
        let directory = test_directory("docx-pdf");
        let input = directory.join("sample.docx");
        docx_rs::Docx::new()
            .add_paragraph(
                docx_rs::Paragraph::new().add_run(docx_rs::Run::new().add_text("Hello DOCX")),
            )
            .build()
            .pack(File::create(&input).unwrap())
            .unwrap();

        assert_eq!(output_formats_for(&input), &["TXT", "PDF"]);

        let output = convert(&input, "pdf", &directory).unwrap();
        assert!(fs::read(&output).unwrap().starts_with(b"%PDF-"));
        assert!(
            pdf_extract::extract_text(&output)
                .unwrap()
                .contains("Hello DOCX")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
