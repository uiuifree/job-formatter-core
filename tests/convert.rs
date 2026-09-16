//! ファイル経由の変換の統合テスト。
#![allow(clippy::unwrap_used)]

use job_formatter_core::adapter::{
    CompanyAdapter, convert_csv_path, convert_json_path, convert_jsonl_path,
};
use job_formatter_core::error::{ConvertError, RecordError};
use job_formatter_core::row::Row;
use job_formatter_core::source::{JSON_RECORD_COLUMN, TableSource};
use serde::{Deserialize, Serialize};
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

/// JSON / JSONL の 1 レコード(JSON オブジェクト)を serde で解釈するアダプタ。
struct JsonAdapter;

#[derive(Deserialize)]
struct JsonLine {
    id: String,
}

impl CompanyAdapter for JsonAdapter {
    type Record = MiniRecord;

    fn company_id(&self) -> &'static str {
        "json"
    }

    fn required_columns(&self) -> &'static [&'static str] {
        &[JSON_RECORD_COLUMN]
    }

    fn convert(&self, row: &Row<'_>) -> Result<Self::Record, RecordError> {
        let line: JsonLine = serde_json::from_str(row.require(JSON_RECORD_COLUMN)?)
            .map_err(|error| RecordError::new(row.record_id(), error.to_string()))?;
        Ok(MiniRecord { id: line.id })
    }
}

#[test]
fn converts_a_jsonl_file_end_to_end() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("jsonl_file");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mini.jsonl");
    // 壊れた JSON 行・空行・項目の型違い(隔離はどちらもアダプタの serde)・正常行
    std::fs::write(
        &path,
        "{\"id\":\"J1\"}\n{broken\n\n{\"id\":1}\n{\"id\":\"J2\"}\n",
    )
    .unwrap();
    let outcome = convert_jsonl_path(&JsonAdapter, &path).unwrap();
    assert_eq!(outcome.records.len(), 2);
    assert_eq!(outcome.records[0].id, "J1");
    assert_eq!(outcome.records[1].id, "J2");
    // 隔離レコードはファイルの物理行番号で指す(空行も数える)
    assert_eq!(outcome.errors.len(), 2);
    assert_eq!(outcome.errors[0].record, "line:2");
    assert_eq!(outcome.errors[1].record, "line:4");

    // 存在しないファイル・UTF-8 でないファイルは読み込みの致命エラー(公開入口から伝播する)
    let err = convert_jsonl_path(&JsonAdapter, &dir.join("missing.jsonl")).unwrap_err();
    assert!(matches!(err, ConvertError::Io { .. }));
    let sjis = dir.join("sjis.jsonl");
    std::fs::write(&sjis, [0x82, 0xa0]).unwrap();
    let err = TableSource::from_jsonl_path(&sjis).unwrap_err();
    assert!(matches!(err, ConvertError::Io { .. }));
    // CSV 用のアダプタ(必須列が違う)を当てるとソース全体が必須列欠落で止まる
    let err = convert_jsonl_path(&MiniAdapter, &path).unwrap_err();
    assert!(matches!(err, ConvertError::MissingColumn { .. }));
}

#[test]
fn converts_a_json_file_end_to_end() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("json_file");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mini.json");
    // 正常・項目の型違い・オブジェクトでない要素(どちらもアダプタの serde で隔離)・正常。
    // 位置は配列の要素番号
    std::fs::write(
        &path,
        "[\n  {\"id\": \"J1\"},\n  {\"id\": 1},\n  null,\n  {\"id\": \"J2\"}\n]\n",
    )
    .unwrap();
    let outcome = convert_json_path(&JsonAdapter, &path, None).unwrap();
    assert_eq!(outcome.records.len(), 2);
    assert_eq!(outcome.records[0].id, "J1");
    assert_eq!(outcome.records[1].id, "J2");
    assert_eq!(outcome.errors.len(), 2);
    assert_eq!(outcome.errors[0].record, "line:2");
    assert_eq!(outcome.errors[1].record, "line:3");

    // 存在しないファイルは読み込み、配列でない最上位はヘッダーの致命エラー(公開入口から伝播する)
    let err = convert_json_path(&JsonAdapter, &dir.join("missing.json"), None).unwrap_err();
    assert!(matches!(err, ConvertError::Io { .. }));
    let wrapped = dir.join("wrapped.json");
    std::fs::write(&wrapped, "{\"items\": [{\"id\": \"W1\"}]}").unwrap();
    let err = convert_json_path(&JsonAdapter, &wrapped, None).unwrap_err();
    assert!(matches!(err, ConvertError::Header { .. }));
    // 包まれた配列は JSON Pointer で選ぶ
    let outcome = convert_json_path(&JsonAdapter, &wrapped, Some("/items")).unwrap();
    assert_eq!(outcome.records.len(), 1);
    assert_eq!(outcome.records[0].id, "W1");
    let sjis = dir.join("sjis.json");
    std::fs::write(&sjis, [0x5b, 0x82, 0xa0, 0x5d]).unwrap();
    let err = convert_json_path(&JsonAdapter, &sjis, None).unwrap_err();
    assert!(matches!(err, ConvertError::Io { .. }));
}
