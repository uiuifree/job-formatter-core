//! 各社アダプタ(取り込み側)の枠組みと変換ドライバ。
//!
//! ドメイン(業界)ごとのレコード型 `Record` に対してジェネリックであり、
//! core は業界知識を持たない。行単位の変換は rayon で並列実行し、入力順を保持する
//! (速度要件: 10 万件を数分以内。変換段は数秒オーダーが目安)。
//! 出力側の双対は [`crate::media`](正規形 → 媒体別出力行)。

use crate::error::{ConvertError, RecordError};
use crate::row::{Row, line_record_id};
use crate::source::TableSource;
use rayon::prelude::*;
use serde::Serialize;

/// 会社(取り込み元)ごとのアダプタ。
/// 列マッピング・社別クレンジングなどの知識はすべて実装側(ドメインクレート)が持つ。
pub trait CompanyAdapter: Sync {
    /// 変換先のレコード型(変換用 JSON の 1 行になる)。
    type Record: Send + Serialize;

    /// 会社(アダプタ)の識別子(例: `"sample"`)。ID は定数として宣言する。
    fn company_id(&self) -> &'static str;

    /// この会社のソースに必須の列名。欠落はソース全体の中止(列マッピング破損)。
    fn required_columns(&self) -> &'static [&'static str];

    /// 1 行をレコードへ変換する。
    ///
    /// # Errors
    ///
    /// 必須欄の欠損・形式不正は [`RecordError`] を返す(この行だけ隔離される)。
    fn convert(&self, row: &Row<'_>) -> Result<Self::Record, RecordError>;
}

/// 変換結果。レコード単位のエラーは隔離し、変換を止めない。
#[derive(Debug)]
pub struct ConvertOutcome<R> {
    /// 変換できたレコード(入力順)。
    pub records: Vec<R>,
    /// 隔離されたレコード単位のエラー(入力順)。
    pub errors: Vec<RecordError>,
}

/// 読み取り済みソースをアダプタで変換する(行単位に並列、入力順を保持)。
///
/// # Errors
///
/// 必須列の欠落は [`ConvertError::MissingColumn`] を返す。
pub fn convert_source<A: CompanyAdapter + ?Sized>(
    adapter: &A,
    source: &TableSource,
) -> Result<ConvertOutcome<A::Record>, ConvertError> {
    source
        .headers()
        .ensure_columns(adapter.required_columns(), source.origin())?;
    // 行単位の変換を並列実行。collect は入力順を保持する。
    // 行番号はソース側が持つ原位置(CSV / Excel では物理行)を使う
    let results: Vec<Result<A::Record, RecordError>> = source
        .rows()
        .par_iter()
        .map(|row| match &row.cells {
            Ok(values) => adapter.convert(&Row::new(source.headers(), values, row.line)),
            Err(reason) => Err(RecordError::new(line_record_id(row.line), reason.clone())),
        })
        .collect();
    let mut outcome = ConvertOutcome {
        records: Vec::new(),
        errors: Vec::new(),
    };
    for result in results {
        match result {
            Ok(record) => outcome.records.push(record),
            Err(error) => outcome.errors.push(error),
        }
    }
    Ok(outcome)
}

/// CSV ファイルを読み取り、アダプタで変換する。
///
/// # Errors
///
/// 読み込み・文字コード・ヘッダー・必須列の問題は [`ConvertError`] を返す。
pub fn convert_csv_path<A: CompanyAdapter + ?Sized>(
    adapter: &A,
    path: &std::path::Path,
) -> Result<ConvertOutcome<A::Record>, ConvertError> {
    let source = TableSource::from_csv_path(path)?;
    convert_source(adapter, &source)
}

/// Excel(xlsx)ファイルを読み取り、アダプタで変換する
/// (`sheet` が `None` なら最初の可視シート)。
///
/// # Errors
///
/// 読み込み・ブック・シート・ヘッダー・必須列の問題は [`ConvertError`] を返す。
#[cfg(feature = "excel")]
pub fn convert_xlsx_path<A: CompanyAdapter + ?Sized>(
    adapter: &A,
    path: &std::path::Path,
    sheet: Option<&str>,
) -> Result<ConvertOutcome<A::Record>, ConvertError> {
    let source = TableSource::from_xlsx_path(path, sheet)?;
    convert_source(adapter, &source)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Debug, Serialize, PartialEq)]
    struct TestRecord {
        id: String,
        name: String,
    }

    struct TestAdapter;

    impl CompanyAdapter for TestAdapter {
        type Record = TestRecord;

        fn company_id(&self) -> &'static str {
            "test"
        }

        fn required_columns(&self) -> &'static [&'static str] {
            &["id", "name"]
        }

        fn convert(&self, row: &Row<'_>) -> Result<Self::Record, RecordError> {
            Ok(TestRecord {
                id: row.require("id")?.to_string(),
                name: row.require("name")?.to_string(),
            })
        }
    }

    #[test]
    fn converts_rows_in_order_and_isolates_errors() {
        let text = "id,name\nJ1,太郎\nJ2,壊れ,行\nJ3,\nJ4,花子\n";
        let source = TableSource::from_csv_bytes(text.as_bytes(), "mem.csv").unwrap();
        let adapter = TestAdapter;
        assert_eq!(adapter.company_id(), "test");
        let outcome = convert_source(&adapter, &source).unwrap();
        // 入力順が保たれる(並列変換でも)
        assert_eq!(
            outcome.records,
            vec![
                TestRecord {
                    id: "J1".to_string(),
                    name: "太郎".to_string()
                },
                TestRecord {
                    id: "J4".to_string(),
                    name: "花子".to_string()
                },
            ]
        );
        assert_eq!(outcome.errors.len(), 2);
        assert!(outcome.errors[0].record.contains("line:3"));
        assert!(outcome.errors[1].record.contains("line:4"));
        assert!(outcome.errors[1].reason.contains("name"));
    }

    #[test]
    fn missing_required_column_aborts_the_source() {
        let source = TableSource::from_csv_bytes(b"id\nJ1\n", "mem.csv").unwrap();
        let err = convert_source(&TestAdapter, &source).unwrap_err();
        assert!(matches!(err, ConvertError::MissingColumn { .. }));
    }

    #[test]
    fn large_input_keeps_order_under_parallelism() {
        // 並列変換でも順序が入力どおりであることを規模を上げて確認する
        use std::fmt::Write as _;
        let mut text = String::from("id,name\n");
        for index in 0..5_000 {
            writeln!(text, "J{index},名前{index}").unwrap();
        }
        let source = TableSource::from_csv_bytes(text.as_bytes(), "mem.csv").unwrap();
        let outcome = convert_source(&TestAdapter, &source).unwrap();
        assert_eq!(outcome.records.len(), 5_000);
        assert!(outcome.errors.is_empty());
        for (index, record) in outcome.records.iter().enumerate() {
            assert_eq!(record.id, format!("J{index}"));
        }
    }

    #[test]
    fn convert_csv_path_propagates_io_error() {
        let err = convert_csv_path(&TestAdapter, std::path::Path::new("/no/such.csv")).unwrap_err();
        assert!(matches!(err, ConvertError::Io { .. }));
    }

    #[cfg(feature = "excel")]
    #[test]
    fn convert_xlsx_path_converts_and_propagates_errors() {
        use rust_xlsxwriter::Workbook;
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write(0, 0, "id").unwrap();
        sheet.write(0, 1, "name").unwrap();
        sheet.write(1, 0, "J1").unwrap();
        sheet.write(1, 1, "太郎").unwrap();
        // プロセス ID を含めて並行実行時のファイル名衝突を避ける
        let path = std::env::temp_dir().join(format!(
            "job_formatter_core_{}_adapter.xlsx",
            std::process::id()
        ));
        workbook.save(&path).unwrap();
        let outcome = convert_xlsx_path(&TestAdapter, &path, None).unwrap();
        assert_eq!(outcome.records.len(), 1);
        assert_eq!(outcome.records[0].id, "J1");
        std::fs::remove_file(&path).unwrap();

        let err = convert_xlsx_path(&TestAdapter, std::path::Path::new("/no/such.xlsx"), None)
            .unwrap_err();
        assert!(matches!(err, ConvertError::Io { .. }));
    }
}
