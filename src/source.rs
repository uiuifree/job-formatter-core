//! 表データソースの読み取り(CSV / Excel / 独自ソース)。
//!
//! 標準の読み口として CSV(UTF-8 / `Shift_JIS` 自動判別)と Excel(xlsx、
//! feature `excel`)を提供する。その他の形式(API レスポンス・固定長など)は
//! [`TableSource::new`] で組み立てれば、同じ変換ドライバ
//! (隔離・並列・順序保持)にそのまま載る。
//! 行単位の問題(列数不一致など)は隔離し、ソース全体の読み取りは止めない。

use crate::error::ConvertError;
use crate::row::Headers;
#[cfg(feature = "excel")]
use calamine::Reader as _;

/// 1 データ行(入力内での原位置つき)。
#[derive(Debug)]
pub struct SourceRow {
    /// 入力内での 1 始まりの行位置。Excel ではシートの実行番号、CSV では
    /// csv パーサの記録位置(引用符内改行を跨いでも実位置。ただしレコード
    /// 直前に空行がある場合はその空行区間の先頭を指すことがある)。
    /// [`TableSource::new`] の独自ソースでは 2 始まりの序数。
    pub line: usize,
    /// セル値(ヘッダーと同数)。行単位で壊れている場合は理由文字列。
    pub cells: Result<Vec<String>, String>,
}

/// 読み取り済みの表データソース(入力形式に依存しない)。
///
/// CSV / Excel の読み取りコンストラクタのほか、[`TableSource::new`] で
/// 任意の形式から組み立てられる。
#[derive(Debug)]
pub struct TableSource {
    /// エラーメッセージ用のソース名(パス等)。
    origin: String,
    headers: Headers,
    rows: Vec<SourceRow>,
}

impl TableSource {
    /// 独自ソース(API レスポンス・固定長など)から組み立てる。
    ///
    /// `rows` はヘッダーを除いたデータ行(入力順)。`Err` は「その行だけ
    /// 壊れている」ことを表し、変換時に隔離される(ソース全体は止まらない)。
    /// 行位置は 2 始まりの序数(ヘッダー = 1 相当)で自動採番される。
    /// `Ok` 行の列数がヘッダーと一致しない場合は、その行を列数不一致として
    /// 隔離する(CSV 経路と同じ扱い)。
    #[must_use]
    pub fn new(
        origin: impl Into<String>,
        headers: Headers,
        rows: Vec<Result<Vec<String>, String>>,
    ) -> Self {
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(index, cells)| SourceRow {
                line: index + 2,
                cells,
            })
            .collect();
        Self::build(origin.into(), headers, rows)
    }

    /// 行長検証つきの内部コンストラクタ(全読み取り経路が通る)。
    fn build(origin: String, headers: Headers, rows: Vec<SourceRow>) -> Self {
        let expected = headers.columns().len();
        let rows = rows
            .into_iter()
            .map(|row| {
                let cells = match row.cells {
                    Ok(values) if values.len() != expected => Err(format!(
                        "列数がヘッダーと一致しません(ヘッダー {expected} 列、実際 {} 列)",
                        values.len()
                    )),
                    other => other,
                };
                SourceRow {
                    line: row.line,
                    cells,
                }
            })
            .collect();
        Self {
            origin,
            headers,
            rows,
        }
    }

    /// CSV ファイルから読み取る。
    ///
    /// # Errors
    ///
    /// 読み込み・文字コード・ヘッダーの問題は [`ConvertError`] を返す。
    pub fn from_csv_path(path: &std::path::Path) -> Result<Self, ConvertError> {
        let origin = path.display().to_string();
        let bytes = std::fs::read(path).map_err(|source| ConvertError::Io {
            path: origin.clone(),
            source,
        })?;
        Self::from_csv_bytes(&bytes, &origin)
    }

    /// CSV のバイト列から読み取る(`origin` はエラーメッセージ用のソース名)。
    ///
    /// # Errors
    ///
    /// 文字コード・ヘッダーの問題は [`ConvertError`] を返す。
    pub fn from_csv_bytes(bytes: &[u8], origin: &str) -> Result<Self, ConvertError> {
        let text = decode(bytes, origin)?;
        Self::parse_csv(text.as_bytes(), origin)
    }

    /// 任意のリーダーから CSV を解析する(文字コード判別済みの入力向け)。
    fn parse_csv<R: std::io::Read>(input: R, origin: &str) -> Result<Self, ConvertError> {
        let mut reader = csv::Reader::from_reader(input);
        let header_record = reader
            .headers()
            .map_err(|error| ConvertError::Header {
                path: origin.to_string(),
                reason: error.to_string(),
            })?
            .clone();
        if header_record.is_empty() {
            // 入力が空(0 バイト等)の場合。MissingColumn(列マッピング破損)と
            // 混同させない
            return Err(ConvertError::Header {
                path: origin.to_string(),
                reason: "ヘッダー行がありません(入力が空)".to_string(),
            });
        }
        let headers = Headers::new(header_record.iter().map(str::to_string).collect());
        let rows = reader
            .into_records()
            .map(|record| match record {
                Ok(row) => SourceRow {
                    line: csv_line(row.position()),
                    cells: Ok(row.iter().map(str::to_string).collect()),
                },
                Err(error) => SourceRow {
                    line: csv_line(error.position()),
                    cells: Err(error.to_string()),
                },
            })
            .collect();
        Ok(Self::build(origin.to_string(), headers, rows))
    }

    /// ソース名(パス等)。
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// ヘッダー。
    #[must_use]
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// データ行(入力順、原位置つき)。壊れている行は理由文字列。
    #[must_use]
    pub fn rows(&self) -> &[SourceRow] {
        &self.rows
    }
}

/// csv クレートの記録位置から物理行番号(1 始まり)を取り出す。
fn csv_line(position: Option<&csv::Position>) -> usize {
    position.map_or(0, |position| {
        usize::try_from(position.line()).unwrap_or(usize::MAX)
    })
}

#[cfg(feature = "excel")]
impl TableSource {
    /// Excel(xlsx)ファイルから読み取る。`sheet` が `None` なら
    /// 最初の可視シート(非表示シートは選ばない)。
    ///
    /// セルの文字列化規則は [`TableSource::from_xlsx_bytes`] を参照。
    ///
    /// # Errors
    ///
    /// 読み込みの問題は [`ConvertError::Io`]、ブック・シート・ヘッダーの問題は
    /// それぞれ [`ConvertError::Workbook`] / [`ConvertError::Sheet`] /
    /// [`ConvertError::Header`] を返す。
    pub fn from_xlsx_path(
        path: &std::path::Path,
        sheet: Option<&str>,
    ) -> Result<Self, ConvertError> {
        let origin = path.display().to_string();
        let mut workbook: calamine::Xlsx<_> =
            calamine::open_workbook(path).map_err(|error| match error {
                calamine::XlsxError::Io(source) => ConvertError::Io {
                    path: origin.clone(),
                    source,
                },
                other => ConvertError::Workbook {
                    path: origin.clone(),
                    reason: other.to_string(),
                },
            })?;
        Self::from_xlsx_workbook(&mut workbook, &origin, sheet)
    }

    /// Excel(xlsx)のバイト列から読み取る(`origin` はエラーメッセージ用の
    /// ソース名、`sheet` が `None` なら最初の可視シート)。
    ///
    /// セルは業界非依存の規則で文字列化する:
    ///
    /// - 数値: 整数値は `123`、それ以外は `123.45`。数値セルは Excel の内部
    ///   表現(IEEE 754 倍精度)に従うため、15 桁を超える整数(ID 等)は
    ///   精度が保証されない。桁数の大きい ID は文字列セルで供給すること
    /// - 日付・時刻: 時刻 0:00:00 の値は `YYYY-MM-DD`、それ以外は
    ///   `YYYY-MM-DDTHH:MM:SS`(ISO 8601)。秒未満は四捨五入する。
    ///   経過時間セル・4 桁年に収まらない値はシリアル値のままの文字列
    /// - 真偽値: `true` / `false`
    /// - 空セル・セルエラー(`#DIV/0!` 等): 空文字列(= 値なし)
    /// - 全セルが空の行: スキップ(CSV の空行と同じ扱い)
    ///
    /// # Errors
    ///
    /// ブック・シート・ヘッダーの問題はそれぞれ [`ConvertError::Workbook`] /
    /// [`ConvertError::Sheet`] / [`ConvertError::Header`] を返す。
    pub fn from_xlsx_bytes(
        bytes: &[u8],
        origin: &str,
        sheet: Option<&str>,
    ) -> Result<Self, ConvertError> {
        let cursor = std::io::Cursor::new(bytes);
        let mut workbook = calamine::Xlsx::new(cursor).map_err(|error| ConvertError::Workbook {
            path: origin.to_string(),
            reason: error.to_string(),
        })?;
        Self::from_xlsx_workbook(&mut workbook, origin, sheet)
    }

    /// 開いたブックの 1 シートを表データへ変換する。
    fn from_xlsx_workbook<R: std::io::Read + std::io::Seek>(
        workbook: &mut calamine::Xlsx<R>,
        origin: &str,
        sheet: Option<&str>,
    ) -> Result<Self, ConvertError> {
        let sheet_name = match sheet {
            Some(name) => name.to_string(),
            None => first_visible_sheet(workbook),
        };
        let range = workbook
            .worksheet_range(&sheet_name)
            .map_err(|error| ConvertError::Sheet {
                path: origin.to_string(),
                sheet: sheet_name.clone(),
                reason: error.to_string(),
            })?;
        // 使用範囲の開始行(0 始まり)。シート先頭の空行・タイトル行の分だけ
        // ヘッダーが下にあるため、行番号はシートの実位置で採番する
        let start_row = range.start().map_or(0, |(row, _)| row);
        let mut rows = range.rows();
        let Some(header_row) = rows.next() else {
            return Err(ConvertError::Header {
                path: origin.to_string(),
                reason: "ヘッダー行がありません".to_string(),
            });
        };
        let headers = Headers::new(header_row.iter().map(cell_to_string).collect());
        let header_line = usize::try_from(start_row).unwrap_or(usize::MAX) + 1;
        let data_rows = rows
            .enumerate()
            .filter_map(|(index, row)| {
                let cells: Vec<String> = row.iter().map(cell_to_string).collect();
                if cells.iter().all(String::is_empty) {
                    // 全セルが空の行はスキップ(CSV の空行と同じ扱い)
                    return None;
                }
                Some(SourceRow {
                    line: header_line + 1 + index,
                    cells: Ok(cells),
                })
            })
            .collect();
        Ok(Self::build(origin.to_string(), headers, data_rows))
    }
}

/// 最初の可視シート名を返す(非表示・超非表示シートは飛ばす)。
#[cfg(feature = "excel")]
fn first_visible_sheet<R: std::io::Read + std::io::Seek>(workbook: &calamine::Xlsx<R>) -> String {
    workbook
        .sheets_metadata()
        .iter()
        .find(|sheet| sheet.visible == calamine::SheetVisible::Visible)
        .map(|sheet| sheet.name.clone())
        .unwrap_or_default()
}

/// UTF-8 → `Shift_JIS` の順に文字列化を試みる。
/// BOM による他エンコーディング(UTF-16 等)への切替は行わない
/// (契約は UTF-8 / `Shift_JIS` のみ。UTF-16 は Encoding エラーで弾く)。
fn decode(bytes: &[u8], origin: &str) -> Result<String, ConvertError> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_string());
    }
    let (decoded, had_errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(bytes);
    if had_errors {
        return Err(ConvertError::Encoding {
            path: origin.to_string(),
        });
    }
    Ok(decoded.into_owned())
}

/// Excel セルを業界非依存の規則で文字列化する(規則は
/// [`TableSource::from_xlsx_bytes`] の doc を参照)。
#[cfg(feature = "excel")]
fn cell_to_string(cell: &calamine::Data) -> String {
    match cell {
        calamine::Data::Empty | calamine::Data::Error(_) => String::new(),
        calamine::Data::String(text) | calamine::Data::DurationIso(text) => text.clone(),
        calamine::Data::DateTimeIso(text) => match text.strip_suffix("T00:00:00") {
            // 真夜中の日時は日付のみへ畳む(シリアル値経路と同じ規則)
            Some(date_only) => date_only.to_string(),
            None => text.clone(),
        },
        calamine::Data::Float(number) => number.to_string(),
        calamine::Data::Int(number) => number.to_string(),
        calamine::Data::Bool(flag) => flag.to_string(),
        calamine::Data::DateTime(value) => excel_datetime_to_string(value),
    }
}

/// Excel シリアル値の日付・時刻を ISO 形式の文字列にする。
#[cfg(feature = "excel")]
fn excel_datetime_to_string(value: &calamine::ExcelDateTime) -> String {
    use chrono::{Datelike as _, Timelike as _};
    // 経過時間セルはシリアル値のまま文字列化する(日時と解釈すると誤りになる)
    if !value.is_datetime() {
        return value.as_f64().to_string();
    }
    let Some(datetime) = value.as_datetime() else {
        // 日時として範囲外の値もシリアル値のまま文字列化する
        return value.as_f64().to_string();
    };
    // 秒未満は四捨五入する(切り捨てると低精度シリアル値で 1 秒早くなる)
    let datetime = if datetime.nanosecond() >= 500_000_000 {
        datetime
            .checked_add_signed(chrono::TimeDelta::seconds(1))
            .unwrap_or(datetime)
    } else {
        datetime
    };
    let year = datetime.year();
    if !(0..=9999).contains(&year) {
        // ISO 8601 の 4 桁年に収まらない値はシリアル値のまま文字列化する
        return value.as_f64().to_string();
    }
    let (month, day) = (datetime.month(), datetime.day());
    let (hour, minute, second) = (datetime.hour(), datetime.minute(), datetime.second());
    if (hour, minute, second) == (0, 0, 0) {
        return format!("{year:04}-{month:02}-{day:02}");
    }
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const CSV_TEXT: &str = "id,name\nJ1,太郎\nJ2,花子\n";

    #[test]
    fn reads_utf8_bytes() {
        let source = TableSource::from_csv_bytes(CSV_TEXT.as_bytes(), "mem.csv").unwrap();
        assert_eq!(source.origin(), "mem.csv");
        assert_eq!(source.rows().len(), 2);
        assert!(source.rows()[0].cells.is_ok());
    }

    #[test]
    fn reads_shift_jis_bytes() {
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode(CSV_TEXT);
        let source = TableSource::from_csv_bytes(&sjis, "sjis.csv").unwrap();
        let row = source.rows()[0].cells.as_ref().unwrap();
        assert_eq!(row[1], "太郎");
    }

    #[test]
    fn undecodable_bytes_are_an_encoding_error() {
        // UTF-8 としても `Shift_JIS` としても不正なバイト列
        let err = TableSource::from_csv_bytes(&[0x82, 0x00, 0xff, 0xff], "bad.csv").unwrap_err();
        assert!(matches!(err, ConvertError::Encoding { .. }));
    }

    #[test]
    fn utf16_bytes_are_an_encoding_error() {
        // BOM 付き UTF-16 は契約外(UTF-8 / Shift_JIS のみ)。BOM スニッフィングで
        // 別エンコーディングとして受理しない
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "id,name\nJ1,太郎\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let err = TableSource::from_csv_bytes(&bytes, "utf16.csv").unwrap_err();
        assert!(matches!(err, ConvertError::Encoding { .. }));
    }

    #[test]
    fn empty_input_is_a_header_error() {
        // 空ファイルは MissingColumn(列マッピング破損)ではなくヘッダー問題として報告
        let err = TableSource::from_csv_bytes(b"", "empty.csv").unwrap_err();
        assert!(matches!(err, ConvertError::Header { .. }));
        assert!(err.to_string().contains("空"));
    }

    #[test]
    fn broken_row_is_isolated_not_fatal() {
        let text = "id,name\nJ1,太郎\nJ2,余分,列\nJ3,花子\n";
        let source = TableSource::from_csv_bytes(text.as_bytes(), "mem.csv").unwrap();
        assert_eq!(source.rows().len(), 3);
        assert!(source.rows()[0].cells.is_ok());
        assert!(source.rows()[1].cells.is_err());
        assert!(source.rows()[2].cells.is_ok());
    }

    #[test]
    fn csv_line_numbers_are_physical() {
        // 空行スキップ・引用符内改行があっても物理行番号を保つ
        // 行 1=ヘッダー、2-3=J1(引用内改行)、4=空行、5=J4、6=J5(列数不一致)
        let text = "id,name\nJ1,\"a\nb\"\n\nJ4,太郎\nJ5,x,y\n";
        let source = TableSource::from_csv_bytes(text.as_bytes(), "mem.csv").unwrap();
        assert_eq!(source.rows().len(), 3);
        assert_eq!(source.rows()[0].line, 2);
        // csv パーサの記録位置は、直前に空行があるとその空行区間の先頭を指す
        // (J4 の物理行は 5 だが 4 と報告される。序数方式の 3 よりも実位置に近い)
        assert_eq!(source.rows()[1].line, 4);
        assert!(source.rows()[2].cells.is_err());
        // 列数不一致エラーの位置は正確な物理行
        assert_eq!(source.rows()[2].line, 6);
    }

    #[test]
    fn unreadable_input_is_a_header_error() {
        struct FailingReader;
        impl std::io::Read for FailingReader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("stream broken"))
            }
        }
        let err = TableSource::parse_csv(FailingReader, "stream.csv").unwrap_err();
        assert!(matches!(err, ConvertError::Header { .. }));
        assert!(err.to_string().contains("stream.csv"));
    }

    #[test]
    fn missing_file_is_io_error() {
        let err = TableSource::from_csv_path(std::path::Path::new("/no/such.csv")).unwrap_err();
        assert!(matches!(err, ConvertError::Io { .. }));
    }

    #[test]
    fn custom_source_rows_are_validated_and_numbered() {
        // 独自ソース(API レスポンス等)の組み立て口。行長不一致は隔離される
        let headers = Headers::new(vec!["id".to_string(), "name".to_string()]);
        let rows = vec![
            Ok(vec!["J1".to_string(), "太郎".to_string()]),
            Ok(vec!["J2".to_string()]),
            Ok(vec!["J3".to_string(), "x".to_string(), "余分".to_string()]),
            Err("upstream error".to_string()),
        ];
        let source = TableSource::new("api:jobs", headers, rows);
        assert_eq!(source.origin(), "api:jobs");
        assert_eq!(source.rows().len(), 4);
        // 序数採番(2 始まり)
        assert_eq!(source.rows()[0].line, 2);
        assert_eq!(source.rows()[3].line, 5);
        assert!(source.rows()[0].cells.is_ok());
        // 短い行・長い行は列数不一致として隔離(無言の欠落・誤誘導エラーにしない)
        let short = source.rows()[1].cells.as_ref().unwrap_err();
        assert!(short.contains("列数がヘッダーと一致しません"));
        assert!(source.rows()[2].cells.is_err());
        // 元から壊れている行はそのまま
        assert_eq!(
            source.rows()[3].cells.as_ref().unwrap_err(),
            "upstream error"
        );
    }
}

#[cfg(all(test, feature = "excel"))]
#[allow(clippy::unwrap_used)]
mod excel_tests {
    use super::*;
    use rust_xlsxwriter::{ExcelDateTime as XlsxDateTime, Format, Workbook};

    fn temp_path(name: &str) -> std::path::PathBuf {
        // プロセス ID を含めて並行実行(cargo test と llvm-cov の同時起動等)の
        // ファイル名衝突を避ける
        std::env::temp_dir().join(format!("job_formatter_core_{}_{name}", std::process::id()))
    }

    /// テスト用の xlsx バイト列(型混在のセルと第 2 シートを持つ)。
    fn sample_xlsx() -> Vec<u8> {
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write(0, 0, "id").unwrap();
        sheet.write(0, 1, "count").unwrap();
        sheet.write(0, 2, "rate").unwrap();
        sheet.write(0, 3, "active").unwrap();
        sheet.write(0, 4, "date").unwrap();
        sheet.write(0, 5, "note").unwrap();
        let date_format = Format::new().set_num_format("yyyy-mm-dd");
        sheet.write(1, 0, "J1").unwrap();
        sheet.write(1, 1, 123).unwrap();
        sheet.write(1, 2, 0.75).unwrap();
        sheet.write(1, 3, true).unwrap();
        sheet
            .write_with_format(
                1,
                4,
                XlsxDateTime::from_ymd(2026, 7, 21).unwrap(),
                &date_format,
            )
            .unwrap();
        // note 列は空セルのまま
        let second = workbook.add_worksheet();
        second.set_name("予備").unwrap();
        second.write(0, 0, "memo").unwrap();
        second.write(1, 0, "第2シート").unwrap();
        workbook.save_to_buffer().unwrap()
    }

    #[test]
    fn reads_first_sheet_with_neutral_cell_strings() {
        let source = TableSource::from_xlsx_bytes(&sample_xlsx(), "mem.xlsx", None).unwrap();
        assert_eq!(source.origin(), "mem.xlsx");
        let row = source.rows()[0].cells.as_ref().unwrap();
        assert_eq!(row[0], "J1");
        assert_eq!(row[1], "123"); // 整数値は小数点なし
        assert_eq!(row[2], "0.75");
        assert_eq!(row[3], "true");
        assert_eq!(row[4], "2026-07-21"); // 時刻 0:00 は日付のみ
        // 空セルは「値なし」= 空文字列
        assert_eq!(row[5], "");
    }

    #[test]
    fn reads_named_sheet() {
        let source =
            TableSource::from_xlsx_bytes(&sample_xlsx(), "mem.xlsx", Some("予備")).unwrap();
        let row = source.rows()[0].cells.as_ref().unwrap();
        assert_eq!(row[0], "第2シート");
    }

    #[test]
    fn default_sheet_skips_hidden_sheets() {
        // 先頭が非表示シートのブックでは、最初の「可視」シートを読む
        let mut workbook = Workbook::new();
        let hidden = workbook.add_worksheet();
        hidden.write(0, 0, "secret").unwrap();
        hidden.write(1, 0, "S1").unwrap();
        hidden.set_hidden(true);
        let visible = workbook.add_worksheet();
        visible.set_name("data").unwrap();
        visible.set_active(true);
        visible.write(0, 0, "id").unwrap();
        visible.write(1, 0, "J1").unwrap();
        let bytes = workbook.save_to_buffer().unwrap();
        let source = TableSource::from_xlsx_bytes(&bytes, "mem.xlsx", None).unwrap();
        assert_eq!(source.headers().columns(), ["id"]);
        assert_eq!(source.rows()[0].cells.as_ref().unwrap()[0], "J1");
    }

    #[test]
    fn xlsx_line_numbers_follow_sheet_rows_and_skip_blank_rows() {
        // ヘッダーがシート 3 行目・データ間に空行がある場合も実シート行で採番
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write(2, 0, "id").unwrap(); // シート行 3
        sheet.write(3, 0, "J1").unwrap(); // シート行 4
        sheet.write(5, 0, "J2").unwrap(); // シート行 6(行 5 は空行)
        let bytes = workbook.save_to_buffer().unwrap();
        let source = TableSource::from_xlsx_bytes(&bytes, "mem.xlsx", None).unwrap();
        // 空行は phantom エラーにせずスキップ(CSV と同じ)
        assert_eq!(source.rows().len(), 2);
        assert_eq!(source.rows()[0].line, 4);
        assert_eq!(source.rows()[1].line, 6);
    }

    #[test]
    fn missing_sheet_is_a_sheet_error() {
        let err =
            TableSource::from_xlsx_bytes(&sample_xlsx(), "mem.xlsx", Some("ghost")).unwrap_err();
        assert!(matches!(err, ConvertError::Sheet { .. }));
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn empty_sheet_is_a_header_error() {
        let mut workbook = Workbook::new();
        workbook.add_worksheet();
        let bytes = workbook.save_to_buffer().unwrap();
        let err = TableSource::from_xlsx_bytes(&bytes, "empty.xlsx", None).unwrap_err();
        assert!(matches!(err, ConvertError::Header { .. }));
    }

    #[test]
    fn garbage_bytes_are_a_workbook_error() {
        let err = TableSource::from_xlsx_bytes(b"not a zip", "bad.xlsx", None).unwrap_err();
        assert!(matches!(err, ConvertError::Workbook { .. }));
    }

    #[test]
    fn xlsx_path_io_and_workbook_errors() {
        // 存在しないファイルは Io
        let err =
            TableSource::from_xlsx_path(std::path::Path::new("/no/such.xlsx"), None).unwrap_err();
        assert!(matches!(err, ConvertError::Io { .. }));
        // 存在するが xlsx でないファイルは Workbook
        let path = temp_path("not_xlsx.xlsx");
        std::fs::write(&path, b"plain text").unwrap();
        let err = TableSource::from_xlsx_path(&path, None).unwrap_err();
        assert!(matches!(err, ConvertError::Workbook { .. }));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn xlsx_path_reads_a_real_file() {
        let path = temp_path("sample.xlsx");
        std::fs::write(&path, sample_xlsx()).unwrap();
        let source = TableSource::from_xlsx_path(&path, None).unwrap();
        assert_eq!(source.rows().len(), 1);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn cell_to_string_covers_all_variants() {
        // xlsx 経由では作りにくい型も含めて、文字列化規則を直接固定する
        assert_eq!(cell_to_string(&calamine::Data::Empty), "");
        assert_eq!(
            cell_to_string(&calamine::Data::Error(calamine::CellErrorType::Div0)),
            ""
        );
        assert_eq!(
            cell_to_string(&calamine::Data::String("テキスト".to_string())),
            "テキスト"
        );
        assert_eq!(cell_to_string(&calamine::Data::Float(123.0)), "123");
        assert_eq!(cell_to_string(&calamine::Data::Float(123.45)), "123.45");
        assert_eq!(cell_to_string(&calamine::Data::Int(-5)), "-5");
        assert_eq!(cell_to_string(&calamine::Data::Bool(false)), "false");
        // ISO 文字列セルも真夜中は日付のみへ畳む(シリアル値経路と同じ規則)
        assert_eq!(
            cell_to_string(&calamine::Data::DateTimeIso(
                "2026-07-21T00:00:00".to_string()
            )),
            "2026-07-21"
        );
        assert_eq!(
            cell_to_string(&calamine::Data::DateTimeIso("2026-07-21T09:00".to_string())),
            "2026-07-21T09:00"
        );
        assert_eq!(
            cell_to_string(&calamine::Data::DurationIso("PT1H".to_string())),
            "PT1H"
        );
    }

    #[test]
    fn datetime_rounds_seconds_and_falls_back_out_of_range() {
        use calamine::{ExcelDateTime, ExcelDateTimeType};
        // 高精度シリアル値: 12:30:00 ちょうど
        let with_time =
            ExcelDateTime::new(46_224.520_833_333_336, ExcelDateTimeType::DateTime, false);
        assert_eq!(
            cell_to_string(&calamine::Data::DateTime(with_time)),
            "2026-07-21T12:30:00"
        );
        // 低精度シリアル値(12:29:59.997)は切り捨てず四捨五入で 12:30:00
        let low_precision =
            ExcelDateTime::new(46_224.520_833_3, ExcelDateTimeType::DateTime, false);
        assert_eq!(
            cell_to_string(&calamine::Data::DateTime(low_precision)),
            "2026-07-21T12:30:00"
        );
        // 経過時間セルはシリアル値のまま
        let duration = ExcelDateTime::new(1.5, ExcelDateTimeType::TimeDelta, false);
        assert_eq!(cell_to_string(&calamine::Data::DateTime(duration)), "1.5");
        // 4 桁年に収まらない値(year 10113)はシリアル値のまま
        let five_digit_year = ExcelDateTime::new(3_000_000.0, ExcelDateTimeType::DateTime, false);
        assert_eq!(
            cell_to_string(&calamine::Data::DateTime(five_digit_year)),
            "3000000"
        );
        // chrono の範囲外はシリアル値のまま
        let out_of_range = ExcelDateTime::new(1.0e10, ExcelDateTimeType::DateTime, false);
        assert_eq!(
            cell_to_string(&calamine::Data::DateTime(out_of_range)),
            "10000000000"
        );
    }
}
