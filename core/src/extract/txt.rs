//! TXT 추출 + 인코딩 감지 (CP949/EUC-KR/UTF-8 대응, `encoding_rs` + `chardetng`).

use super::{ContentExtractor, DocumentInfo, ExtractError, ExtractionResult};
use chardetng::EncodingDetector;
use std::fs;

pub struct TxtExtractor;

impl ContentExtractor for TxtExtractor {
    fn supports(&self, ext: &str) -> bool {
        ext.eq_ignore_ascii_case("txt")
    }

    fn extract(&self, document: &DocumentInfo) -> Result<ExtractionResult, ExtractError> {
        let bytes = fs::read(&document.path)?;
        Ok(ExtractionResult {
            body: decode_text(&bytes),
        })
    }
}

fn decode_text(bytes: &[u8]) -> String {
    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    let encoding = detector.guess(None, true);
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn decodes_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.txt");
        std::fs::File::create(&path)
            .unwrap()
            .write_all("채권 발행 절차".as_bytes())
            .unwrap();

        let result = TxtExtractor
            .extract(&DocumentInfo {
                path,
                extension: "txt".into(),
            })
            .unwrap();
        assert_eq!(result.body, "채권 발행 절차");
    }

    #[test]
    fn decodes_euc_kr() {
        let (bytes, _, had_errors) = encoding_rs::EUC_KR.encode("채권 발행 절차");
        assert!(!had_errors);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.txt");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();

        let result = TxtExtractor
            .extract(&DocumentInfo {
                path,
                extension: "txt".into(),
            })
            .unwrap();
        assert_eq!(result.body, "채권 발행 절차");
    }

    #[test]
    fn supports_only_txt() {
        assert!(TxtExtractor.supports("txt"));
        assert!(TxtExtractor.supports("TXT"));
        assert!(!TxtExtractor.supports("pdf"));
    }
}
