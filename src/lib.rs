//! # job-formatter-core
//!
//! *Industry-agnostic job-feed formatting engine: company CSV → normalized
//! JSONL → per-media output rows. Parallel, order-preserving, fault-isolating.*
//!
//! 求人フィード連携の**業界非依存基盤**。「各社 CSV → 正規形(JSONL)」の取り込みと、
//! その双対である「正規形 → 媒体別出力行」の整形、それぞれの**仕組み**を担う。
//!
//! ```text
//! 会社A CSV ─┐                          ┌─→ 媒体X向け出力行
//! 会社B CSV ─┼─→ 正規形レコード(JSONL)─┼─→ 媒体Y向け出力行
//! 会社C CSV ─┘                          └─→ 媒体Z向け出力行
//! ```
//!
//! ## 責務境界(横展開の設計)
//!
//! - **core(本クレート)**: アダプタの枠組み([`adapter::CompanyAdapter`])・
//!   CSV 読取(UTF-8 / `Shift_JIS` 自動判別)・Excel 読取(feature `excel`、既定で有効)・
//!   独自ソースの組み立て口([`source::TableSource::new`])・列名アクセス・
//!   レコード単位のエラー隔離・並列変換・JSONL 出力・媒体整形の枠組み
//!   ([`media::MediaFormatter`])。レコード型に対してジェネリックで、
//!   **業界知識・媒体知識を一切持たない**
//! - **ドメインクレート**(業界ごと): 正規形レコード型の定義、各社アダプタ実装
//!   (列マッピング・社別クレンジング)、CLI。タグ付けなど後段の加工もドメイン側に置く
//! - **媒体実装**(出力先ごと、呼び出し側 — バーティカルの media モジュール等):
//!   [`media::MediaFormatter`] の実装。マスタ ID 変換・出力形式・媒体検証などの
//!   知識はすべて媒体側が持つ
//!
//! ## 速度
//!
//! 行単位の変換([`adapter::convert_source`])・媒体整形([`media::format_records`])・
//! シリアライズ([`output::write_jsonl`])は rayon で並列実行し、入力順を保持する。
//! 直列なのはファイル I/O のみ。
//!
//! ## 使い方
//!
//! ### 取り込み(各社 CSV → 正規形)
//!
//! ```
//! use job_formatter_core::adapter::{CompanyAdapter, convert_source};
//! use job_formatter_core::error::RecordError;
//! use job_formatter_core::row::Row;
//! use job_formatter_core::source::TableSource;
//! use serde::Serialize;
//!
//! #[derive(Serialize)]
//! struct MyJob {
//!     job_no: String,
//!     title: String,
//! }
//!
//! struct MyAgentAdapter;
//!
//! impl CompanyAdapter for MyAgentAdapter {
//!     type Record = MyJob;
//!
//!     fn company_id(&self) -> &'static str {
//!         "my-agent"
//!     }
//!
//!     fn required_columns(&self) -> &'static [&'static str] {
//!         &["求人番号", "タイトル"]
//!     }
//!
//!     fn convert(&self, row: &Row<'_>) -> Result<Self::Record, RecordError> {
//!         Ok(MyJob {
//!             job_no: row.require("求人番号")?.to_string(),
//!             title: row.require("タイトル")?.to_string(),
//!         })
//!     }
//! }
//!
//! # fn main() -> Result<(), job_formatter_core::error::ConvertError> {
//! let csv = "求人番号,タイトル\nJ1,スタッフ募集\nJ2,\n";
//! let source = TableSource::from_csv_bytes(csv.as_bytes(), "agent.csv")?;
//! // Excel なら TableSource::from_xlsx_path、独自形式なら TableSource::new で同じ流れに載る
//! let outcome = convert_source(&MyAgentAdapter, &source)?;
//! assert_eq!(outcome.records.len(), 1);   // J2 はタイトル空で隔離
//! assert_eq!(outcome.errors.len(), 1);
//! # Ok(())
//! # }
//! ```
//!
//! ### 媒体整形(正規形 → 媒体別 JSONL)
//!
//! ```
//! use job_formatter_core::error::RecordError;
//! use job_formatter_core::media::{MediaFormatter, format_records};
//! use job_formatter_core::output::write_jsonl;
//! use serde::Serialize;
//!
//! struct MyJob {
//!     job_no: String,
//!     employments: Vec<String>,
//! }
//!
//! #[derive(Serialize)]
//! struct MyMediaRow {
//!     job_no: String,
//!     employment: String,
//! }
//!
//! struct MyMediaFormatter;
//!
//! impl MediaFormatter for MyMediaFormatter {
//!     type Record = MyJob;
//!     type Output = MyMediaRow;
//!
//!     fn media_id(&self) -> &'static str {
//!         "my-media"
//!     }
//!
//!     fn format(&self, record: &MyJob) -> Result<Vec<MyMediaRow>, RecordError> {
//!         // 雇用形態ごとに 1 行へ展開。0 行なら「この媒体では出力しない」(選別)
//!         Ok(record
//!             .employments
//!             .iter()
//!             .map(|employment| MyMediaRow {
//!                 job_no: record.job_no.clone(),
//!                 employment: employment.clone(),
//!             })
//!             .collect())
//!     }
//! }
//!
//! # fn main() -> Result<(), job_formatter_core::error::OutputError> {
//! let jobs = vec![
//!     MyJob { job_no: "J1".into(), employments: vec!["正社員".into(), "パート".into()] },
//!     MyJob { job_no: "J2".into(), employments: vec![] },
//! ];
//! let outcome = format_records(&MyMediaFormatter, &jobs);
//! assert_eq!(outcome.rows.len(), 2);   // J1 は 2 行に展開、J2 は選別(エラーではない)
//! assert!(outcome.errors.is_empty());
//! let mut jsonl = Vec::new();
//! write_jsonl(&mut jsonl, &outcome.rows)?;
//! # Ok(())
//! # }
//! ```

pub mod adapter;
pub mod error;
pub mod media;
pub mod output;
pub mod row;
pub mod source;
