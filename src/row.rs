//! ヘッダー付き表データ行への列名アクセス。
//!
//! アダプタ実装が列インデックスを意識せず、列名で安全に値を取り出すための薄い層。
//! 入力形式(CSV / Excel / 独自ソース)には依存しない。
//! 列名と値は前後空白を除去して扱い、空文字列は「値なし」として扱う
//! (例外は [`Row::raw_payload`]。監査用に生値を保持する)。

use crate::error::{ConvertError, RecordError};
use std::collections::BTreeMap;

/// 行番号からレコード識別子(`line:{n}`)を作る。
/// 書式の単一情報源([`Row::record_id`] と変換ドライバの双方が使う)。
pub(crate) fn line_record_id(line: usize) -> String {
    format!("line:{line}")
}

/// 表データのヘッダー(列名 → インデックス)。
#[derive(Debug, Clone)]
pub struct Headers {
    columns: Vec<String>,
    index: BTreeMap<String, usize>,
}

impl Headers {
    /// 列名の並びから構築する。列名は前後空白を除去して扱い、
    /// 同名列が複数ある場合は最初の列を採用する。
    #[must_use]
    pub fn new(columns: Vec<String>) -> Self {
        let columns: Vec<String> = columns
            .into_iter()
            .map(|name| name.trim().to_string())
            .collect();
        let mut index = BTreeMap::new();
        for (position, name) in columns.iter().enumerate() {
            index.entry(name.clone()).or_insert(position);
        }
        Self { columns, index }
    }

    /// 列名の並び(入力順、trim 済み)。
    #[must_use]
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// 列が存在するか(非致命の存在確認)。
    #[must_use]
    pub fn contains(&self, column: &str) -> bool {
        self.index.contains_key(column)
    }

    /// アダプタが必須と宣言した列がすべて存在するか検証する。
    ///
    /// # Errors
    ///
    /// 欠落があれば [`ConvertError::MissingColumn`] を返す(ソース全体を中止)。
    pub fn ensure_columns(&self, required: &[&str], path: &str) -> Result<(), ConvertError> {
        for column in required {
            if !self.contains(column) {
                return Err(ConvertError::MissingColumn {
                    path: path.to_string(),
                    column: (*column).to_string(),
                });
            }
        }
        Ok(())
    }

    fn position(&self, column: &str) -> Option<usize> {
        self.index.get(column).copied()
    }
}

/// ヘッダーに紐づいた 1 データ行。
#[derive(Debug)]
pub struct Row<'a> {
    headers: &'a Headers,
    values: &'a [String],
    line: usize,
}

impl<'a> Row<'a> {
    /// ヘッダー・データ行・行番号(入力内の 1 始まりの位置)から構築する。
    #[must_use]
    pub fn new(headers: &'a Headers, values: &'a [String], line: usize) -> Self {
        Self {
            headers,
            values,
            line,
        }
    }

    /// 入力内での行位置(1 始まり)。CSV / Excel では物理行番号。
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    /// レコード識別子(`line:{n}`)。エラー報告用。
    #[must_use]
    pub fn record_id(&self) -> String {
        line_record_id(self.line)
    }

    fn value(&self, column: &str) -> Option<&str> {
        let position = self.headers.position(column)?;
        self.values.get(position).map(|value| value.trim())
    }

    /// 必須欄の値を取り出す。列が無い・値が空ならレコードエラー。
    ///
    /// # Errors
    ///
    /// 列がヘッダーに存在しない場合は「列がありません」、値が空の場合は
    /// 「必須欄が空」の [`RecordError`] を返す(このレコードを隔離)。
    /// 前者が大量に出る場合は列マッピング(`required_columns` の宣言漏れ)を疑うこと。
    pub fn require(&self, column: &str) -> Result<&str, RecordError> {
        if !self.headers.contains(column) {
            return Err(RecordError::new(
                self.record_id(),
                format!("列がありません: {column}"),
            ));
        }
        match self.value(column) {
            Some(value) if !value.is_empty() => Ok(value),
            _ => Err(RecordError::new(
                self.record_id(),
                format!("必須欄が空: {column}"),
            )),
        }
    }

    /// 任意欄の値を取り出す(列が無い・空文字列は `None`)。
    #[must_use]
    pub fn optional(&self, column: &str) -> Option<String> {
        self.value(column)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    /// 区切り文字で分割した複数値欄(空要素は除去、各要素は trim 済み)。
    #[must_use]
    pub fn list(&self, column: &str, separators: &[char]) -> Vec<String> {
        self.value(column)
            .map(|value| {
                value
                    .split(|ch| separators.contains(&ch))
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 行全体を監査用 JSON(列名 → 値)にする。
    ///
    /// 監査用に値は生のまま(trim なし)保持する。重複列は最初の列を採用し
    /// (`require` と同じ規約)、値が欠けている列は `null` になる。
    #[must_use]
    pub fn raw_payload(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (position, column) in self.headers.columns.iter().enumerate() {
            if map.contains_key(column) {
                // 重複列は最初の列を採用(require と同じ規約)
                continue;
            }
            let value = self
                .values
                .get(position)
                .map_or(serde_json::Value::Null, |value| {
                    serde_json::Value::from(value.as_str())
                });
            map.insert(column.clone(), value);
        }
        serde_json::Value::Object(map)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn values(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    fn headers() -> Headers {
        Headers::new(values(&["id", "name", "tags", "note"]))
    }

    #[test]
    fn require_returns_trimmed_value_or_record_error() {
        let headers = headers();
        let data = values(&[" J1 ", "太郎", "a/b", ""]);
        let row = Row::new(&headers, &data, 2);
        assert_eq!(row.line(), 2);
        assert_eq!(row.require("id").unwrap(), "J1");
        let err = row.require("note").unwrap_err();
        assert_eq!(err.record, "line:2");
        assert!(err.reason.contains("必須欄が空"));
        assert!(err.reason.contains("note"));
    }

    #[test]
    fn require_distinguishes_missing_column_from_empty_value() {
        let headers = headers();
        let data = values(&["J1", "n", "t", "x"]);
        let row = Row::new(&headers, &data, 2);
        // 列マッピング問題(列が存在しない)は空値と区別して報告する
        let err = row.require("ghost").unwrap_err();
        assert!(err.reason.contains("列がありません"));
        assert!(err.reason.contains("ghost"));
    }

    #[test]
    fn headers_trim_column_names_and_expose_accessors() {
        // ヘッダーの前後空白(手作業編集の CSV に頻出)は除去して照合する
        let headers = Headers::new(values(&["id ", " name", "tags"]));
        assert_eq!(headers.columns(), ["id", "name", "tags"]);
        assert!(headers.contains("name"));
        assert!(!headers.contains(" name"));
        assert!(!headers.contains("ghost"));
        assert!(headers.ensure_columns(&["id", "name"], "a.csv").is_ok());
    }

    #[test]
    fn optional_treats_empty_as_none() {
        let headers = headers();
        let data = values(&["J1", "  ", "a/b", "備考"]);
        let row = Row::new(&headers, &data, 3);
        assert_eq!(row.optional("name"), None);
        assert_eq!(row.optional("note").as_deref(), Some("備考"));
        assert_eq!(row.optional("ghost"), None);
    }

    #[test]
    fn list_splits_and_drops_empty_items() {
        let headers = headers();
        let data = values(&["J1", "n", " A / B 、/、C ", "x"]);
        let row = Row::new(&headers, &data, 4);
        assert_eq!(row.list("tags", &['/', '、']), vec!["A", "B", "C"]);
        assert!(row.list("ghost", &['/']).is_empty());
    }

    #[test]
    fn raw_payload_maps_all_columns_with_raw_values() {
        let headers = headers();
        let data = values(&[" J1 ", "n", "t", ""]);
        let row = Row::new(&headers, &data, 5);
        let payload = row.raw_payload();
        // 監査用のため trim せず生値を保持する
        assert_eq!(payload["id"], " J1 ");
        assert_eq!(payload["note"], "");
    }

    #[test]
    fn raw_payload_uses_first_duplicate_and_null_for_missing() {
        let headers = Headers::new(values(&["id", "id", "name"]));
        let data = values(&["first", "second"]);
        let row = Row::new(&headers, &data, 2);
        let payload = row.raw_payload();
        // 重複列は require と同じ first-wins、欠けている列は null
        assert_eq!(payload["id"], "first");
        assert_eq!(payload["name"], serde_json::Value::Null);
    }

    #[test]
    fn duplicated_header_uses_first_column() {
        let headers = Headers::new(values(&["id", "id"]));
        let data = values(&["first", "second"]);
        let row = Row::new(&headers, &data, 2);
        assert_eq!(row.require("id").unwrap(), "first");
    }

    #[test]
    fn ensure_columns_reports_missing_column() {
        let headers = headers();
        assert!(headers.ensure_columns(&["id", "name"], "a.csv").is_ok());
        let err = headers
            .ensure_columns(&["id", "salary"], "a.csv")
            .unwrap_err();
        assert!(err.to_string().contains("salary"));
    }
}
