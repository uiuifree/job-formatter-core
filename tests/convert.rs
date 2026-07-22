//! ファイル経由の変換の統合テスト。
#![allow(clippy::unwrap_used)]

use job_formatter_core::adapter::{CompanyAdapter, convert_csv_path};
use job_formatter_core::error::RecordError;
use job_formatter_core::row::Row;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
struct MiniRecord {
    id: String,
}

struct MiniAdapter;

impl CompanyAdapter for MiniAdapter {
    type Record = MiniRecord;

    fn company_id(&self) -> &'static str {
        "mini"
    }

    fn required_columns(&self) -> &'static [&'static str] {
        &["id"]
    }

    fn convert(&self, row: &Row<'_>) -> Result<Self::Record, RecordError> {
        Ok(MiniRecord {
            id: row.require("id")?.to_string(),
        })
    }
}

#[test]
fn converts_a_csv_file_end_to_end() {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mini.csv");
    std::fs::write(&path, "id,name\nJ1,太郎\n,空\n").unwrap();
    let outcome = convert_csv_path(&MiniAdapter, &path).unwrap();
    assert_eq!(outcome.records.len(), 1);
    assert_eq!(outcome.records[0].id, "J1");
    assert_eq!(outcome.errors.len(), 1);
}
