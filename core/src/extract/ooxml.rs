//! DOCX / PPTX 추출 (`zip` + `quick-xml`).
//!
//! 두 포맷 모두 텍스트가 `<*:t>` 요소에, 문단 경계가 `<*:p>` 종료 태그에 있다
//! (DOCX는 `w:t`/`w:p`, PPTX는 `a:t`/`a:p` — 네임스페이스만 다르고 로컬 이름은
//! 같아 `local_name()`으로 접두사를 벗기면 동일한 파서를 재사용할 수 있다).

use super::{ContentExtractor, DocumentInfo, ExtractError, ExtractionResult};
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::fs::File;
use std::io::Read;

pub struct DocxExtractor;

impl ContentExtractor for DocxExtractor {
    fn supports(&self, ext: &str) -> bool {
        ext.eq_ignore_ascii_case("docx")
    }

    fn extract(&self, document: &DocumentInfo) -> Result<ExtractionResult, ExtractError> {
        let mut archive = open_zip(&document.path)?;
        let xml = read_entry(&mut archive, "word/document.xml")?;
        Ok(ExtractionResult {
            body: extract_paragraph_text(&xml),
        })
    }
}

pub struct PptxExtractor;

impl ContentExtractor for PptxExtractor {
    fn supports(&self, ext: &str) -> bool {
        ext.eq_ignore_ascii_case("pptx")
    }

    fn extract(&self, document: &DocumentInfo) -> Result<ExtractionResult, ExtractError> {
        let mut archive = open_zip(&document.path)?;

        let mut slide_names: Vec<String> = archive
            .file_names()
            .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
            .map(|name| name.to_string())
            .collect();
        slide_names.sort_by_key(|name| slide_number(name).unwrap_or(u32::MAX));

        let mut body = String::new();
        for name in slide_names {
            let xml = read_entry(&mut archive, &name)?;
            let text = extract_paragraph_text(&xml);
            if !text.is_empty() {
                body.push_str(&text);
            }
        }

        Ok(ExtractionResult { body })
    }
}

fn open_zip(path: &std::path::Path) -> Result<zip::ZipArchive<File>, ExtractError> {
    let file = File::open(path)?;
    zip::ZipArchive::new(file).map_err(|e| ExtractError::Parse(e.to_string()))
}

fn read_entry(archive: &mut zip::ZipArchive<File>, name: &str) -> Result<String, ExtractError> {
    let mut entry = archive
        .by_name(name)
        .map_err(|e| ExtractError::Parse(e.to_string()))?;
    let mut xml = String::new();
    entry.read_to_string(&mut xml)?;
    Ok(xml)
}

/// `ppt/slides/slide12.xml` -> `12`. 슬라이드 순서를 파일명 자릿수가 아니라
/// 실제 숫자로 정렬하기 위함 (`slide10`이 `slide2`보다 사전식으로 앞서는 문제 방지).
fn slide_number(name: &str) -> Option<u32> {
    let file_name = name.rsplit('/').next()?;
    let digits: String = file_name.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn extract_paragraph_text(xml: &str) -> String {
    // 텍스트를 trim하지 않는다 — DOCX/PPTX는 <w:t>/<a:t> 안의 공백(어절 경계)이
    // 의미를 가지므로, 여기서 지우면 "채권"+"발행"이 "채권발행"으로 붙어버린다.
    let mut reader = Reader::from_str(xml);

    let mut body = String::new();
    let mut current_paragraph = String::new();
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().local_name().as_ref() == b"t" => {
                in_text = true;
            }
            Ok(Event::Text(e)) if in_text => {
                if let Ok(decoded) = e.decode() {
                    if let Ok(text) = unescape(&decoded) {
                        current_paragraph.push_str(&text);
                    }
                }
            }
            Ok(Event::End(e)) if e.name().local_name().as_ref() == b"t" => {
                in_text = false;
            }
            Ok(Event::End(e)) if e.name().local_name().as_ref() == b"p" => {
                if !current_paragraph.is_empty() {
                    body.push_str(&current_paragraph);
                    body.push('\n');
                    current_paragraph.clear();
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break, // 손상된 XML은 여기까지 모은 텍스트만 반환 (META 강등은 호출부 책임)
            _ => {}
        }
    }

    if !current_paragraph.is_empty() {
        body.push_str(&current_paragraph);
        body.push('\n');
    }

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn write_zip_entry(zip: &mut zip::ZipWriter<File>, name: &str, contents: &str) {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }

    fn sample_docx(path: &std::path::Path) {
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        write_zip_entry(
            &mut zip,
            "word/document.xml",
            r#"<w:document xmlns:w="http://x"><w:body>
                <w:p><w:r><w:t>채권</w:t></w:r><w:r><w:t> 발행</w:t></w:r></w:p>
                <w:p><w:r><w:t>절차</w:t></w:r></w:p>
            </w:body></w:document>"#,
        );
        zip.finish().unwrap();
    }

    fn sample_pptx(path: &std::path::Path, slide_count: u32) {
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for n in 1..=slide_count {
            write_zip_entry(
                &mut zip,
                &format!("ppt/slides/slide{n}.xml"),
                &format!(
                    r#"<p:sld xmlns:a="http://x"><p:cSld><p:spTree><p:sp><p:txBody>
                        <a:p><a:r><a:t>슬라이드{n}</a:t></a:r></a:p>
                    </p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
                ),
            );
        }
        zip.finish().unwrap();
    }

    #[test]
    fn extracts_docx_paragraphs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.docx");
        sample_docx(&path);

        let result = DocxExtractor
            .extract(&DocumentInfo {
                path,
                extension: "docx".into(),
            })
            .unwrap();

        assert_eq!(result.body, "채권 발행\n절차\n");
    }

    #[test]
    fn extracts_pptx_slides_in_numeric_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.pptx");
        sample_pptx(&path, 11); // slide10/slide11이 slide2보다 사전식으로 앞서는 케이스 포함

        let result = PptxExtractor
            .extract(&DocumentInfo {
                path,
                extension: "pptx".into(),
            })
            .unwrap();

        let expected: String = (1..=11).map(|n| format!("슬라이드{n}\n")).collect();
        assert_eq!(result.body, expected);
    }

    #[test]
    fn supports_correct_extensions() {
        assert!(DocxExtractor.supports("docx"));
        assert!(!DocxExtractor.supports("pptx"));
        assert!(PptxExtractor.supports("PPTX"));
        assert!(!PptxExtractor.supports("docx"));
    }
}
