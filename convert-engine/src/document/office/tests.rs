use super::*;
use std::fs;
use std::io::Write;

struct TestDir(std::path::PathBuf);
impl TestDir {
    fn new() -> Self {
        let name = format!(
            "office-conversion-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(name);
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn save(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn sample() -> OfficeDocument {
    let normal = TextStyle::default();
    let emphatic = TextStyle {
        font: "바탕".into(),
        size: 27,
        bold: true,
        italic: true,
        underline: true,
        strike: true,
        color: 0x1234ab,
    };
    let paragraph = Paragraph {
        runs: vec![
            TextRun {
                text: "한글과 English 😀\t첫째\n둘째 ".into(),
                style: normal.clone(),
            },
            TextRun {
                text: "강조한 글자".into(),
                style: emphatic,
            },
        ],
        style: ParagraphStyle {
            align: 3,
            before: 120,
            after: 80,
            left: 240,
            right: 120,
            indent: -60,
            page_break: false,
        },
    };
    let make_cell = |row, col, row_span, col_span, text: &str| Cell {
        row,
        col,
        row_span,
        col_span,
        width: 2000 * col_span as u32,
        paragraphs: vec![Paragraph {
            runs: vec![TextRun {
                text: text.into(),
                style: normal.clone(),
            }],
            style: ParagraphStyle::default(),
        }],
    };
    OfficeDocument {
        blocks: vec![
            Block::Paragraph(paragraph),
            Block::Paragraph(Paragraph::default()),
            Block::Table(Table {
                rows: 2,
                cols: 3,
                cells: vec![
                    make_cell(0, 0, 2, 1, "세로 병합"),
                    make_cell(0, 1, 1, 2, "가로 병합"),
                    make_cell(1, 1, 1, 1, "왼쪽"),
                    make_cell(1, 2, 1, 1, "오른쪽"),
                ],
            }),
            Block::Paragraph(Paragraph {
                runs: vec![TextRun {
                    text: "표 뒤의 문단".into(),
                    style: normal,
                }],
                style: ParagraphStyle {
                    page_break: true,
                    ..ParagraphStyle::default()
                },
            }),
        ],
        page: Page::default(),
    }
}

#[test]
fn docx_hwp_round_trip_keeps_unicode_styles_paragraphs_and_merged_tables() {
    let directory = TestDir::new();
    let expected = sample();
    let docx = directory.save("source.docx", &write_docx(&expected).unwrap());
    assert_eq!(read_docx(&docx).unwrap(), expected);
    let hwp = directory.save("result.hwp", &docx_to_hwp(&docx).unwrap());
    assert_eq!(hwp::read(&hwp).unwrap(), expected);
    let result = directory.save("result.docx", &hwp_to_docx(&hwp).unwrap());
    assert_eq!(read_docx(&result).unwrap(), expected);
    // The result is a complete DOCX package readable by a separate DOCX parser.
    docx_rs::read_docx(&fs::read(&result).unwrap()).unwrap();
}

#[test]
fn public_conversion_lists_the_new_formats_and_does_not_overwrite() {
    let directory = TestDir::new();
    let docx = directory.save("input.DOCX", &write_docx(&sample()).unwrap());
    assert!(crate::document::output_formats_for(&docx).contains(&"HWP"));
    let first = crate::document::convert(&docx, "hwp", &directory.0).unwrap();
    let original = fs::read(&first).unwrap();
    let second = crate::document::convert(&docx, "hwp", &directory.0).unwrap();
    assert_ne!(first, second);
    assert_eq!(fs::read(&first).unwrap(), original);
    assert_eq!(
        crate::document::output_formats_for(&first),
        &["DOCX", "HWPX"]
    );
    let output = crate::document::convert(&first, "docx", &directory.0).unwrap();
    assert_eq!(read_docx(&output).unwrap(), sample());
}

fn minimal_docx(xml: &str) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "word/document.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(xml.as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
}

#[test]
fn unsupported_content_and_corrupt_input_do_not_create_partial_output() {
    let directory = TestDir::new();
    let docx = directory.save("picture.docx", &minimal_docx("<w:document xmlns:w='http://schemas.openxmlformats.org/wordprocessingml/2006/main'><w:body><w:p><w:r><w:t>그림 앞</w:t><w:drawing/></w:r></w:p></w:body></w:document>"));
    assert!(!docx_supported(&docx));
    assert!(crate::document::convert(&docx, "hwp", &directory.0).is_err());
    assert!(!directory.0.join("picture.hwp").exists());
    let corrupt = directory.save("corrupt.hwp", b"invalid");
    assert!(!hwp_supported(&corrupt));
    assert!(crate::document::convert(&corrupt, "docx", &directory.0).is_err());
    assert!(!directory.0.join("corrupt.docx").exists());
    let protected = directory.save("protected.hwp", &hwp::write(&sample()).unwrap());
    let mut file = cfb::open_rw(&protected).unwrap();
    let mut header = limited_read(file.open_stream("/FileHeader").unwrap()).unwrap();
    header[36] |= 2;
    file.create_stream("/FileHeader")
        .unwrap()
        .write_all(&header)
        .unwrap();
    drop(file);
    assert!(!hwp_supported(&protected));
    assert!(
        hwp_to_docx(&protected)
            .unwrap_err()
            .to_string()
            .contains("암호")
    );
}

#[test]
fn docx_inherited_styles_and_empty_paragraphs_are_preserved() {
    use std::io::Write;
    let directory = TestDir::new();
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "word/document.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"<w:document xmlns:w='http://schemas.openxmlformats.org/wordprocessingml/2006/main'><w:body><w:p><w:pPr><w:pStyle w:val='Child'/></w:pPr><w:r><w:t>Styled</w:t></w:r></w:p><w:p/></w:body></w:document>").unwrap();
    zip.start_file("word/styles.xml", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"<w:styles xmlns:w='http://schemas.openxmlformats.org/wordprocessingml/2006/main'><w:style w:styleId='Base'><w:rPr><w:b/><w:sz w:val='30'/></w:rPr><w:pPr><w:jc w:val='center'/></w:pPr></w:style><w:style w:styleId='Child'><w:basedOn w:val='Base'/><w:rPr><w:i/></w:rPr></w:style></w:styles>").unwrap();
    let input = directory.save("styles.docx", &zip.finish().unwrap().into_inner());
    let model = read_docx(&input).unwrap();
    let Block::Paragraph(p) = &model.blocks[0] else {
        panic!()
    };
    assert_eq!(p.style.align, 3);
    assert!(p.runs[0].style.bold);
    assert!(p.runs[0].style.italic);
    assert_eq!(p.runs[0].style.size, 30);
    let output = directory.save("styles.hwp", &docx_to_hwp(&input).unwrap());
    assert_eq!(hwp::read(&output).unwrap(), model);
}

#[test]
fn table_at_document_start_and_extended_text_records_round_trip() {
    let directory = TestDir::new();
    let mut expected = sample();
    expected.blocks.swap(0, 2);
    let Block::Paragraph(p) = &mut expected.blocks[2] else {
        panic!()
    };
    p.runs[0].text = "긴 문단😀\t".repeat(1500);
    let input = directory.save("long.docx", &write_docx(&expected).unwrap());
    let output = directory.save("long.hwp", &docx_to_hwp(&input).unwrap());
    assert_eq!(hwp::read(&output).unwrap(), expected);
}

#[test]
fn invalid_page_and_paragraph_values_return_errors() {
    let directory = TestDir::new();
    for properties in [
        "<w:sectPr><w:pgMar w:left='4294967295'/></w:sectPr>",
        "<w:p><w:pPr><w:ind w:firstLine='-2147483648'/></w:pPr></w:p>",
    ] {
        let xml = format!(
            "<w:document xmlns:w='http://schemas.openxmlformats.org/wordprocessingml/2006/main'><w:body>{properties}</w:body></w:document>"
        );
        let path = directory.save("invalid.docx", &minimal_docx(&xml));
        assert!(!docx_supported(&path));
        assert!(docx_to_hwp(&path).is_err());
    }
}

#[test]
fn all_four_hwpx_conversion_routes_preserve_the_document() {
    let directory = TestDir::new();
    let expected = sample();
    let docx = directory.save("source.DOCX", &write_docx(&expected).unwrap());
    let hwpx = crate::convert(crate::Category::Document, &docx, "HWPX", &directory.0).unwrap();
    assert_eq!(hwpx::read(&hwpx).unwrap(), expected);
    assert_eq!(crate::document::output_formats_for(&hwpx), &["DOCX", "HWP"]);
    assert!(crate::document::OUTPUT_FORMATS.contains(&"HWPX"));
    assert!(crate::document::output_formats_for(&docx).contains(&"HWPX"));
    let restored = crate::document::convert(&hwpx, "docx", &directory.0).unwrap();
    assert_eq!(read_docx(&restored).unwrap(), expected);
    docx_rs::read_docx(&fs::read(&restored).unwrap()).unwrap();
    let hwp = crate::document::convert(&hwpx, "hwp", &directory.0).unwrap();
    assert_eq!(hwp::read(&hwp).unwrap(), expected);
    let second = crate::document::convert(&hwp, "hwpx", &directory.0).unwrap();
    assert_ne!(hwpx, second);
    assert_eq!(hwpx::read(&second).unwrap(), expected);
    // HWPX's MIME signature is the first, uncompressed ZIP member.
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&hwpx).unwrap()).unwrap();
    let first = archive.by_index(0).unwrap();
    assert_eq!(first.name(), "mimetype");
    assert_eq!(first.compression(), zip::CompressionMethod::Stored);
}

fn edit_hwpx(source: &[u8], edit: impl Fn(&str, String) -> String) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(source)).unwrap();
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..archive.len() {
        let file = archive.by_index(i).unwrap();
        let name = file.name().to_string();
        let data = String::from_utf8(limited_read(file).unwrap()).unwrap();
        zip.start_file(&name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(edit(&name, data).as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn hwpx_escaping_empty_paragraphs_table_first_and_long_text_round_trip() {
    let directory = TestDir::new();
    let mut expected = sample();
    expected.blocks.swap(0, 2);
    let Block::Paragraph(p) = &mut expected.blocks[2] else {
        panic!()
    };
    p.runs[0].text = "<&\"'>한글😀\t\n\t".repeat(1500);
    p.runs[0].style.font = "Font <&\"'".into();
    let input = directory.save("special.hwpx", &hwpx::write(&expected).unwrap());
    assert_eq!(hwpx::read(&input).unwrap(), expected);
    let docx = directory.save("special.docx", &hwpx_to_docx(&input).unwrap());
    assert_eq!(read_docx(&docx).unwrap(), expected);
}

#[test]
fn hwpx_invalid_and_unsupported_documents_create_no_partial_outputs() {
    let directory = TestDir::new();
    let source = hwpx::write(&sample()).unwrap();
    let changes = [
        ("mimetype", "application/hwp+zip", "application/zip"),
        ("Contents/header.xml", "height=\"1100\"", "height=\"0\""),
        (
            "Contents/section0.xml",
            "charPrIDRef=\"1\"",
            "charPrIDRef=\"999\"",
        ),
        ("Contents/section0.xml", "<hp:t>", "<hp:pic/><hp:t>"),
        ("Contents/section0.xml", "colCount=\"1\"", "colCount=\"2\""),
        (
            "Contents/section0.xml",
            "colSpan=\"2\"",
            "colSpan=\"9999999\"",
        ),
        ("Contents/section0.xml", "width=\"59530\"", "width=\"0\""),
        (
            "Contents/section0.xml",
            "masterPageCnt=\"0\"",
            "masterPageCnt=\"1\"",
        ),
        (
            "Contents/content.hpf",
            "Contents/section0.xml",
            "../section0.xml",
        ),
        (
            "META-INF/manifest.xml",
            "/>",
            "><odf:encryption-data/></odf:manifest>",
        ),
    ];
    for (index, (entry, from, to)) in changes.iter().enumerate() {
        let altered = edit_hwpx(&source, |name, text| {
            if name == *entry {
                assert!(text.contains(from));
                text.replace(from, to)
            } else {
                text
            }
        });
        let input = directory.save(&format!("bad{index}.hwpx"), &altered);
        assert!(!hwpx_supported(&input), "case {index}");
        assert!(crate::document::output_formats_for(&input).is_empty());
        for target in ["docx", "hwp"] {
            assert!(
                crate::document::convert(&input, target, &directory.0).is_err(),
                "case {index}"
            );
            assert!(!directory.0.join(format!("bad{index}.{target}")).exists());
        }
    }
}

#[test]
fn hwpx_reads_manifest_order_and_relative_paths_for_multiple_sections() {
    let directory = TestDir::new();
    let model = sample();
    let source = hwpx::write(&model).unwrap();
    let source = edit_hwpx(&source, |name, text| {
        match name {
        "Contents/header.xml" => text.replace("secCnt=\"1\"", "secCnt=\"2\""),
        "Contents/content.hpf" => text.replace("href=\"Contents/header.xml\"", "href=\"header.xml\"")
            .replace("href=\"Contents/section0.xml\"", "href=\"section0.xml\"")
            .replace("</opf:manifest>", "<opf:item id=\"other-body\" href=\"body.xml\" media-type=\"application/xml\"/></opf:manifest>")
            .replace("</opf:spine>", "<opf:itemref idref=\"other-body\"/></opf:spine>"),
        _ => text,
    }
    });
    let mut archive = zip::ZipArchive::new(Cursor::new(&source)).unwrap();
    let second = limited_read(archive.by_name("Contents/section0.xml").unwrap()).unwrap();
    drop(archive);
    // append preserves the existing package and adds a non-numbered body part.
    let mut zip = zip::ZipWriter::new_append(Cursor::new(source)).unwrap();
    zip.start_file(
        "Contents/body.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(&second).unwrap();
    let input = directory.save("sections.hwpx", &zip.finish().unwrap().into_inner());
    let mut expected = model.clone();
    let mut second = model.blocks;
    let Block::Paragraph(first) = &mut second[0] else {
        panic!()
    };
    first.style.page_break = true;
    expected.blocks.extend(second);
    assert_eq!(hwpx::read(&input).unwrap(), expected);
    let output = directory.save("sections.hwp", &hwpx_to_hwp(&input).unwrap());
    assert_eq!(hwp::read(&output).unwrap(), expected);
}

#[test]
#[ignore = "set HWPX_VALIDATION_DIR to a folder containing independent blank.hwpx and SimpleTable.hwpx fixtures"]
fn independent_hwpx_samples_and_validation_exports() {
    let directory = std::path::PathBuf::from(
        std::env::var_os("HWPX_VALIDATION_DIR").expect("HWPX_VALIDATION_DIR"),
    );
    for name in ["blank", "SimpleTable"] {
        let input = directory.join(format!("{name}.hwpx"));
        let model = hwpx::read(&input).unwrap();
        if name == "SimpleTable" {
            let table = model
                .blocks
                .iter()
                .find_map(|b| {
                    if let Block::Table(t) = b {
                        Some(t)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!((table.rows, table.cols), (3, 3));
            assert!(
                table
                    .cells
                    .iter()
                    .any(|c| c.row_span == 2 && c.col_span == 2)
            );
        }
        let hwp = directory.join(format!("{name}-converted.hwp"));
        fs::write(&hwp, hwpx_to_hwp(&input).unwrap()).unwrap();
        assert_eq!(hwp::read(&hwp).unwrap(), model);
        let docx = directory.join(format!("{name}-converted.docx"));
        fs::write(&docx, hwpx_to_docx(&input).unwrap()).unwrap();
        assert_eq!(read_docx(&docx).unwrap(), model);
    }
    for name in ["sample1", "SimplePicture", "MultiColumn"] {
        assert!(!hwpx_supported(&directory.join(format!("{name}.hwpx"))));
    }
    fs::write(
        directory.join("native-output.hwpx"),
        hwpx::write(&sample()).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "set HWP_VALIDATION_DIR to a folder containing independent blank.hwp, plain-table.hwp and 표.hwp fixtures"]
fn independent_hwp_samples_and_validation_exports() {
    let directory = std::path::PathBuf::from(
        std::env::var_os("HWP_VALIDATION_DIR").expect("HWP_VALIDATION_DIR"),
    );
    fs::write(
        directory.join("native-output.hwp"),
        hwp::write(&sample()).unwrap(),
    )
    .unwrap();
    let blank = hwp::read(&directory.join("blank.hwp")).unwrap();
    assert!(!blank.blocks.is_empty());
    assert!(
        hwp::read(&directory.join("표.hwp"))
            .unwrap_err()
            .to_string()
            .contains("캡션")
    );
    let table = hwp::read(&directory.join("plain-table.hwp")).unwrap();
    assert!(table.blocks.iter().any(|b| matches!(b, Block::Table(_))));
    for (name, model) in [("blank", &blank), ("plain-table", &table)] {
        let output = directory.join(format!("{name}-converted.hwpx"));
        fs::write(
            &output,
            hwp_to_hwpx(&directory.join(format!("{name}.hwp"))).unwrap(),
        )
        .unwrap();
        assert_eq!(hwpx::read(&output).unwrap(), *model);
    }
    fs::write(
        directory.join("external-table.docx"),
        write_docx(&table).unwrap(),
    )
    .unwrap();
    let exported = hwp::write(&sample()).unwrap();
    fs::write(directory.join("native-output.hwp"), &exported).unwrap();
    fs::write(
        directory.join("native-output.docx"),
        write_docx(&sample()).unwrap(),
    )
    .unwrap();
}
