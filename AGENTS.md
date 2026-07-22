# AGENTS.md — job-formatter-core 開発ガイド(人間・AI エージェント共通)

求人フィード連携の**業界非依存基盤**。「各社 CSV → 正規形(JSONL)」の取り込みと、
その双対である「正規形 → 媒体別出力行」の整形、それぞれの**仕組み**を担う。
レコード型に対してジェネリックで、**業界知識・媒体知識を一切持たない**。

## 責務境界(横展開の設計)

- **core(本リポジトリ)**: `CompanyAdapter` trait / 表データ読取(CSV: UTF-8・Shift_JIS
  自動判別、Excel: feature `excel` 既定有効、独自形式: `TableSource::new` で組み立て)/
  列名アクセス(`Row`)/ レコード単位のエラー隔離 / 並列変換 / JSONL 出力 /
  媒体整形の仕組み(`MediaFormatter` trait + 隔離つき一括整形ドライバ)
- **ドメインクレート**(job-formatter-kaigo / -kangoshi …): 変換用レコード型・各社アダプタ・CLI。
  タグ付け(tagpipe-*)・給与数値化・エリア解決もドメイン側の後段
- **媒体実装**(呼び出し側 — バーティカルの media モジュール等): `MediaFormatter` 実装。媒体の知識
  (マスタ ID 表・出力形式の型・展開ルール・媒体検証・UUID 等の運用状態)は
  すべて媒体側が持ち、core に入れない
- ドメイン語(介護・職種名など)・媒体語(具体的な出力先名など)を
  core のコード・API・テストに入れない

## 速度(必須要件)

- 10 万件を数分以内。行単位の変換(`convert_source`)とシリアライズ(`write_jsonl`)は
  rayon で並列、直列なのはファイル I/O のみ。順序は入力どおり保持する
- 新しい処理を足すときも「行単位で並列化できる形」を保つこと

## コマンド

```bash
cargo test
cargo test --no-default-features    # CSV のみ構成(CI は clippy も同構成で実行)
cargo fmt --check
cargo clippy --all-targets -- -D warnings    # pedantic 有効
cargo doc --no-deps    # rustdoc lint 強制
cargo machete
# 「2」は複数テストバイナリ統合時の LLVM 集計アーティファクト分(実未カバー行はゼロ)
cargo llvm-cov --fail-under-functions 100 --fail-uncovered-lines 2
```

## 絶対規約

- `unwrap()` / `expect()` は本番パス禁止(テストのみ)。`unsafe` 禁止。デッドコード禁止
- 公開 API に日本語 doc コメント必須。エラーは thiserror でステージ別(致命 `ConvertError` /
  隔離 `RecordError`)。依存追加は理由を Cargo.toml コメントに明記
- レコード単位の問題でソース全体を止めない(隔離して継続)

## 構成

| パス | 役割 |
|---|---|
| `src/adapter.rs` | `CompanyAdapter` trait・並列変換ドライバ・`ConvertOutcome` |
| `src/media.rs` | `MediaFormatter` trait・一括整形ドライバ・`MediaOutcome` |
| `src/source.rs` | 表データ読取: `TableSource`(CSV / Excel / 独自ソースの組み立て口・行単位エラー隔離) |
| `src/row.rs` | 列名アクセス(`require` / `optional` / `list` / `raw_payload`) |
| `src/output.rs` | JSONL 出力(並列シリアライズ) |
| `src/error.rs` | `ConvertError`(致命)/ `RecordError`(隔離)/ `OutputError` |
