# job-formatter-core

[![Crates.io](https://img.shields.io/crates/v/job-formatter-core?style=flat-square)](https://crates.io/crates/job-formatter-core)
[![CI](https://img.shields.io/github/actions/workflow/status/uiuifree/job-formatter-core/ci.yaml?style=flat-square&label=CI)](https://github.com/uiuifree/job-formatter-core/actions/workflows/ci.yaml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=flat-square)](#license)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange?style=flat-square)](Cargo.toml)

**Industry-agnostic job-feed formatting engine in Rust.** Convert per-company
CSV / Excel feeds into normalized JSONL, then fan normalized records out into
per-media (job board / ad platform) output rows — in parallel, with per-record
fault isolation and automatic UTF-8 / Shift_JIS detection. A focused ETL
building block for job-board and recruitment data pipelines.

[日本語版はこちら](#日本語)

```text
Company A CSV ─┐                         ┌─→ rows for Media X
Company B CSV ─┼─→ normalized JSONL hub ─┼─→ rows for Media Y
Company C CSV ─┘                         └─→ rows for Media Z
```

Instead of maintaining N×M converters (N companies × M media), you implement
N `CompanyAdapter`s and M `MediaFormatter`s around a normalized-record hub.
This crate ships the **mechanism only** — record types and business knowledge
(master-ID tables, validation rules, tagging) live on the caller side: your
domain crates and their media implementations.

## Features

- **Generic over your record type** — core has zero industry or media
  knowledge; bring your own serde types
- **CSV and Excel (xlsx) ingestion built in** — CSV with encoding
  auto-detection (UTF-8 first, then Shift_JIS, common in Japanese data feeds);
  Excel behind the default-on `excel` cargo feature, with documented
  industry-neutral cell-to-string rules (dates become ISO 8601)
- **Custom sources without ceremony** — any other format (JSON API, fixed
  width, …) plugs into the same isolation/parallel driver by building a
  `TableSource` from plain headers + rows
- **Record-level error isolation** — a broken row is quarantined with its
  reason and the batch continues; only fatal problems (unreadable file,
  missing required columns) abort a source
- **Parallel and deterministic** — row conversion, media formatting, and JSON
  serialization run on [rayon]; output order always matches input order
  (100k records in seconds)
- **JSONL in, JSONL out** — stream-friendly, diff-friendly, partially
  reprocessable

[rayon]: https://crates.io/crates/rayon

## How it works

```text
Ingestion
  CSV / Excel / custom rows
       │  TableSource (headers + rows)
       ▼
  convert_source(adapter)      parallel per record · order preserved
       ├─▶ records ──▶ write_jsonl ──▶ normalized JSONL
       └─▶ errors       quarantined per record — the batch continues

Media formatting
  normalized records
       ▼
  format_records(formatter)    parallel per record · 1 record → 0..n rows
       ├─▶ rows ──▶ write_jsonl ──▶ per-media JSONL
       └─▶ errors       quarantined
```

Only file I/O is sequential; conversion, formatting, and serialization all run
on rayon while output order stays deterministic. A fatal problem (unreadable
file, missing required column) aborts the source; anything record-level is
quarantined into `errors` with its reason.

## Quick start

### 1. Ingest: company CSV → normalized records

Implement `CompanyAdapter` (column mapping and company-specific cleansing),
then run the parallel driver:

```rust
use job_formatter_core::adapter::{CompanyAdapter, convert_csv_path};
use job_formatter_core::error::RecordError;
use job_formatter_core::output::write_jsonl;
use job_formatter_core::row::Row;
use serde::Serialize;

#[derive(Serialize)]
struct MyJob {
    job_no: String,
    title: String,
}

struct MyAgentAdapter;

impl CompanyAdapter for MyAgentAdapter {
    type Record = MyJob;

    fn company_id(&self) -> &'static str {
        "my-agent"
    }

    fn required_columns(&self) -> &'static [&'static str] {
        &["求人番号", "タイトル"]
    }

    fn convert(&self, row: &Row<'_>) -> Result<Self::Record, RecordError> {
        Ok(MyJob {
            job_no: row.require("求人番号")?.to_string(),
            title: row.require("タイトル")?.to_string(),
        })
    }
}

let outcome = convert_csv_path(&MyAgentAdapter, std::path::Path::new("feed.csv"))?;
// Excel feed?  convert_xlsx_path(&MyAgentAdapter, path, None)      — first visible sheet
//              convert_xlsx_path(&MyAgentAdapter, path, Some("求人")) — named sheet
// Custom feed? build a TableSource::new(origin, headers, rows) and use convert_source
// outcome.records: converted records, input order preserved
// outcome.errors:  quarantined records with reasons — the batch never dies
let mut file = std::fs::File::create("normalized.jsonl")?;
write_jsonl(&mut file, &outcome.records)?;
```

### 2. Format: normalized records → per-media rows

Implement `MediaFormatter` (master-ID mapping, media validation, row
expansion). One record maps to 0..n output rows:

```rust
use job_formatter_core::error::RecordError;
use job_formatter_core::media::{MediaFormatter, format_records};
use serde::Serialize;

#[derive(Serialize)]
struct MyMediaRow {
    job_no: String,
    employment: String,
}

struct MyMediaFormatter;

impl MediaFormatter for MyMediaFormatter {
    type Record = MyJob;
    type Output = MyMediaRow;

    fn media_id(&self) -> &'static str {
        "my-media"
    }

    fn format(&self, record: &MyJob) -> Result<Vec<MyMediaRow>, RecordError> {
        // Ok(vec![])          → not published to this media (filtering, not an error)
        // Ok(vec![a, b, ...]) → one row per search axis (expansion)
        // Err(RecordError)    → only this record is quarantined; the batch continues
        todo!("media-specific mapping and validation")
    }
}

// `records` is the ingestion result from step 1 (`outcome.records`)
let outcome = format_records(&MyMediaFormatter, &records);
// outcome.rows / outcome.errors — same isolation model as ingestion
```

Complete runnable examples live in the crate documentation (doctests in
`src/lib.rs`).

## API overview

| Module | What it provides |
|---|---|
| `adapter` | `CompanyAdapter` trait + parallel conversion driver (`convert_csv_path` / `convert_source`) |
| `media` | `MediaFormatter` trait + parallel formatting driver (`format_records`), 1 record → 0..n rows |
| `source` | `TableSource`: CSV / Excel readers + constructor for custom sources, per-row isolation |
| `row` | Column access by name: `require` / `optional` / `list` / `raw_payload` |
| `output` | `write_jsonl`: parallel serialization, ordered writes |
| `error` | `ConvertError` (fatal, per source) / `RecordError` (isolated, per record) / `OutputError` |

## Design notes and non-goals

- Core holds **mechanism only**: no master-ID tables, no salary parsing, no
  tagging, no upload I/O. Those belong to your domain crates and their media
  implementations (e.g. a `media` module on the caller side).
- No async runtime, no networking, no CLI — this is a library for building
  your own pipeline binaries.
- Excel support is a default-on cargo feature; opt out with
  `default-features = false` if you only ingest CSV.
- Development gates (tests, clippy pedantic, 100% function coverage) are
  documented in [AGENTS.md](AGENTS.md).

---

<a id="日本語"></a>

## 日本語

求人フィード連携の**業界非依存基盤**です。「各社 CSV → 正規形(JSONL)」の取り込みと、
その双対である「正規形 → 媒体別出力行」の整形、それぞれの**仕組み**を提供します。

```text
会社A CSV ─┐                          ┌─→ 媒体X向け出力行
会社B CSV ─┼─→ 正規形レコード(JSONL)─┼─→ 媒体Y向け出力行
会社C CSV ─┘                          └─→ 媒体Z向け出力行
```

N 社 × M 媒体の直接変換(N×M 実装)を避け、正規形ハブを挟んで
N 個の `CompanyAdapter` と M 個の `MediaFormatter` に分解します。
core が持つのは**仕組みのみ**で、レコード型・業務知識(マスタ ID 表・
検証ルール・タグ付け)は利用側のドメインクレート / 媒体実装
(バーティカルの media モジュール等)に置きます。

### 特徴

- **レコード型にジェネリック**: core は業界知識・媒体知識を一切持たない
- **CSV / Excel(xlsx)を標準サポート**: CSV は UTF-8 → Shift_JIS の順で
  文字コード自動判別。Excel は既定有効の cargo feature `excel`(シート指定可、
  セルの文字列化規則は業界非依存で明文化。日付は ISO 8601)
- **独自ソースも同じ流れに**: その他の形式(API・固定長など)は
  `TableSource::new` で組み立てれば、隔離・並列・順序保持のドライバにそのまま載る
- **レコード単位のエラー隔離**: 壊れた行は理由つきで隔離して継続。
  ソース全体を止めるのは読込・文字コード・必須列欠落などの致命問題のみ
- **並列かつ決定的**: 変換・媒体整形・シリアライズは rayon で並列、
  出力順は常に入力順(10 万件で数秒オーダー)
- **JSONL 入出力**: ストリーム処理・差分レビュー・部分再処理がしやすい

### 使い方

上の Quick start のとおり、取り込み側は `CompanyAdapter` を実装して
`convert_csv_path` → `write_jsonl`、媒体側は `MediaFormatter` を実装して
`format_records` を呼びます。実行可能な完全例は API ドキュメント
(`src/lib.rs` の doctest)を参照してください。

### 開発

ビルド・テスト・品質ゲートは [AGENTS.md](AGENTS.md) を参照。

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
