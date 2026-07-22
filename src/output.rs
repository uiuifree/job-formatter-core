//! 変換用 JSON(JSONL)の出力。
//!
//! シリアライズは rayon で並列実行し、書き込みだけを入力順に直列で行う(速度要件)。
//! JSONL を採用する理由: ストリーム処理・差分レビュー・部分再処理がしやすい。

use crate::error::OutputError;
use rayon::prelude::*;
use serde::Serialize;
use std::io::Write;

/// レコード列を JSONL(1 行 = 1 レコード)で書き出す。
///
/// 1 レコードにつき 1 回の書き込みにまとめるが、大量件数を `File` へ直接
/// 書く場合は [`std::io::BufWriter`] で包むことを推奨する。
///
/// # Errors
///
/// シリアライズ失敗(データ起因)は [`OutputError::Serialize`]、
/// 書き込み失敗は [`OutputError::Io`] を返す。
pub fn write_jsonl<W: Write, T: Serialize + Sync>(
    writer: &mut W,
    records: &[T],
) -> Result<(), OutputError> {
    // シリアライズはメモリ内で並列に行い、順序どおりに集める。
    // 改行も行バッファへ含め、1 レコード 1 write にする
    let lines: Vec<Result<Vec<u8>, serde_json::Error>> = records
        .par_iter()
        .map(|record| {
            serde_json::to_vec(record).map(|mut line| {
                line.push(b'\n');
                line
            })
        })
        .collect();
    for line in lines {
        writer.write_all(&line?)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde::Serializer;

    #[derive(Serialize)]
    struct Item {
        id: usize,
    }

    #[test]
    fn writes_one_line_per_record_in_order() {
        let items: Vec<Item> = (0..100).map(|id| Item { id }).collect();
        let mut buffer = Vec::new();
        write_jsonl(&mut buffer, &items).unwrap();
        let text = String::from_utf8(buffer).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 100);
        assert_eq!(lines[0], r#"{"id":0}"#);
        assert_eq!(lines[99], r#"{"id":99}"#);
        assert!(text.ends_with('\n'));
    }

    /// 常にシリアライズに失敗する型(エラー経路テスト用)。
    struct FailingSerialize;

    impl Serialize for FailingSerialize {
        fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("intentional"))
        }
    }

    #[test]
    fn serialize_failure_is_reported_as_serialize_error() {
        let mut buffer = Vec::new();
        let err = write_jsonl(&mut buffer, &[FailingSerialize]).unwrap_err();
        assert!(matches!(err, OutputError::Serialize(_)));
    }

    /// 常に書き込みに失敗する Writer(エラー経路テスト用)。
    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("full"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_failure_is_reported_as_io_error() {
        let mut writer = FailingWriter;
        assert!(writer.flush().is_ok());
        let err = write_jsonl(&mut writer, &[Item { id: 1 }]).unwrap_err();
        assert!(matches!(err, OutputError::Io(_)));
    }
}
