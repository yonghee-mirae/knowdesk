//! XLSX 추출 (`calamine`). 모든 시트를 순회하며 셀 값을 행 단위 텍스트로 이어붙인다.

use super::{ContentExtractor, DocumentInfo, ExtractError, ExtractionResult};
use calamine::{open_workbook, Reader, Xlsx};
use std::fs::File;
use std::io::BufReader;

pub struct XlsxExtractor;

impl ContentExtractor for XlsxExtractor {
    fn supports(&self, ext: &str) -> bool {
        ext.eq_ignore_ascii_case("xlsx")
    }

    fn extract(&self, document: &DocumentInfo) -> Result<ExtractionResult, ExtractError> {
        let mut workbook = open_workbook::<Xlsx<BufReader<File>>, _>(&document.path)
            .map_err(|e| ExtractError::Parse(e.to_string()))?;

        let mut body = String::new();
        for (_sheet_name, range) in workbook.worksheets() {
            for row in range.rows() {
                let cells: Vec<String> = row
                    .iter()
                    .map(|cell| cell.to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !cells.is_empty() {
                    body.push_str(&cells.join(" "));
                    body.push('\n');
                }
            }
        }

        Ok(ExtractionResult { body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_xlsxwriter::Workbook;

    fn sample_xlsx(path: &std::path::Path) {
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "채권").unwrap();
        sheet.write_string(0, 1, "발행").unwrap();
        sheet.write_string(1, 0, "절차").unwrap();
        workbook.save(path).unwrap();
    }

    #[test]
    fn extracts_cell_text_row_by_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.xlsx");
        sample_xlsx(&path);

        let result = XlsxExtractor
            .extract(&DocumentInfo {
                path,
                extension: "xlsx".into(),
            })
            .unwrap();

        assert_eq!(result.body, "채권 발행\n절차\n");
    }

    #[test]
    fn supports_only_xlsx() {
        assert!(XlsxExtractor.supports("xlsx"));
        assert!(XlsxExtractor.supports("XLSX"));
        assert!(!XlsxExtractor.supports("txt"));
    }
}
