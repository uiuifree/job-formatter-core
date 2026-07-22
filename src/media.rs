//! 媒体(出力先)ごとの整形の枠組みと一括整形ドライバ。
//!
//! [`CompanyAdapter`](crate::adapter::CompanyAdapter)(各社 CSV → 正規形)の
//! 双対として「正規形 → 媒体別の出力行」を担う。マスタ ID 変換・媒体検証・
//! 展開ルールなどの媒体知識は core は持たず、すべて実装側(呼び出し側の媒体実装)の領分。
//! 整形はレコード単位に並列実行し、入力順を保持する(速度要件は取り込み側と同じ)。

use crate::error::RecordError;
use rayon::prelude::*;
use serde::Serialize;

/// 媒体(出力先)ごとのフォーマッタ。
/// マスタ ID 変換・媒体検証・展開ルールなどの知識はすべて実装側が持つ。
pub trait MediaFormatter: Sync {
    /// 入力となる正規形レコード型。
    type Record: Sync;
    /// 媒体別の出力行型(JSONL 1 行になる)。
    type Output: Send + Serialize;

    /// 媒体(フォーマッタ)の識別子(例: `"sample"`)。ID は定数として宣言する。
    fn media_id(&self) -> &'static str;

    /// 1 レコードを媒体別の出力行(0..n 行)へ整形する。
    ///
    /// - `Ok(vec![])` は「この媒体では出力しない」(選別。エラーではない)
    /// - 複数行は展開(1 レコード → 検索軸ごとの行など)を表す
    ///
    /// # Errors
    ///
    /// 媒体要件を満たせないレコードは [`RecordError`] を返す(このレコードだけ隔離)。
    /// レコード識別子は正規形レコード自身の ID から実装側が設定する。
    fn format(&self, record: &Self::Record) -> Result<Vec<Self::Output>, RecordError>;
}

/// 整形結果。レコード単位のエラーは隔離し、整形を止めない。
#[derive(Debug)]
pub struct MediaOutcome<O> {
    /// 整形できた出力行(レコードの入力順、レコード内は `format` の返却順)。
    pub rows: Vec<O>,
    /// 隔離されたレコード単位のエラー(入力順)。
    pub errors: Vec<RecordError>,
}

/// 正規形レコード列を媒体別に一括整形する(レコード単位に並列、順序保持)。
#[must_use]
pub fn format_records<F: MediaFormatter + ?Sized>(
    formatter: &F,
    records: &[F::Record],
) -> MediaOutcome<F::Output> {
    // convert_source と同じ形: par_iter + collect は入力順を保持する
    let results: Vec<Result<Vec<F::Output>, RecordError>> = records
        .par_iter()
        .map(|record| formatter.format(record))
        .collect();
    let mut outcome = MediaOutcome {
        rows: Vec::new(),
        errors: Vec::new(),
    };
    for result in results {
        match result {
            Ok(rows) => outcome.rows.extend(rows),
            Err(error) => outcome.errors.push(error),
        }
    }
    outcome
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::output::write_jsonl;

    struct TestRecord {
        id: String,
        variants: Vec<String>,
    }

    impl TestRecord {
        fn new(id: &str, variants: &[&str]) -> Self {
            Self {
                id: id.to_string(),
                variants: variants.iter().map(ToString::to_string).collect(),
            }
        }
    }

    #[derive(Debug, Serialize, PartialEq)]
    struct TestRow {
        id: String,
        variant: String,
    }

    struct TestFormatter;

    impl MediaFormatter for TestFormatter {
        type Record = TestRecord;
        type Output = TestRow;

        fn media_id(&self) -> &'static str {
            "test"
        }

        fn format(&self, record: &TestRecord) -> Result<Vec<TestRow>, RecordError> {
            if record.id.starts_with("NG") {
                return Err(RecordError::new(record.id.clone(), "要件を満たさない"));
            }
            // variants が空 = この媒体では出力しない(選別)
            Ok(record
                .variants
                .iter()
                .map(|variant| TestRow {
                    id: record.id.clone(),
                    variant: variant.clone(),
                })
                .collect())
        }
    }

    fn row(id: &str, variant: &str) -> TestRow {
        TestRow {
            id: id.to_string(),
            variant: variant.to_string(),
        }
    }

    #[test]
    fn expands_records_in_order_and_isolates_errors() {
        let records = vec![
            TestRecord::new("J1", &["a", "b"]),
            TestRecord::new("NG1", &["c"]),
            TestRecord::new("J2", &[]),
            TestRecord::new("NG2", &["d"]),
            TestRecord::new("J3", &["e"]),
        ];
        let formatter = TestFormatter;
        assert_eq!(formatter.media_id(), "test");
        let outcome = format_records(&formatter, &records);
        // 展開行はレコードの入力順、レコード内は format の返却順
        assert_eq!(
            outcome.rows,
            vec![row("J1", "a"), row("J1", "b"), row("J3", "e")]
        );
        // 選別(空 Vec)はエラーに入らず、隔離エラーは入力順
        assert_eq!(outcome.errors.len(), 2);
        assert_eq!(outcome.errors[0].record, "NG1");
        assert_eq!(outcome.errors[1].record, "NG2");
        assert!(outcome.errors[0].reason.contains("要件"));
    }

    #[test]
    fn large_input_keeps_order_under_parallelism() {
        // 並列整形でも順序が入力どおりであることを規模を上げて確認する
        let records: Vec<TestRecord> = (0..5_000)
            .map(|index| TestRecord::new(&format!("J{index}"), &["x", "y"]))
            .collect();
        let outcome = format_records(&TestFormatter, &records);
        assert_eq!(outcome.rows.len(), 10_000);
        assert!(outcome.errors.is_empty());
        for (index, pair) in outcome.rows.chunks(2).enumerate() {
            assert_eq!(
                pair,
                [
                    row(&format!("J{index}"), "x"),
                    row(&format!("J{index}"), "y")
                ]
            );
        }
    }

    #[test]
    fn rows_serialize_to_jsonl_with_existing_writer() {
        let records = vec![TestRecord::new("J1", &["a"]), TestRecord::new("J2", &["b"])];
        let outcome = format_records(&TestFormatter, &records);
        let mut buffer = Vec::new();
        write_jsonl(&mut buffer, &outcome.rows).unwrap();
        let text = String::from_utf8(buffer).unwrap();
        assert_eq!(
            text,
            "{\"id\":\"J1\",\"variant\":\"a\"}\n{\"id\":\"J2\",\"variant\":\"b\"}\n"
        );
    }
}
