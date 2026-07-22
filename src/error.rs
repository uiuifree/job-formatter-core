//! 取り込みのエラー型。
//!
//! ソース全体が読めない致命エラー([`ConvertError`])と、
//! 隔離してパイプラインを止めないレコード単位のエラー([`RecordError`])を区別する。

/// ソース全体に関わる致命エラー(このソースの変換を中止する)。
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    /// 入力ファイルの読み込みに失敗した。
    #[error("入力の読み込みに失敗: {path}: {source}")]
    Io {
        /// 対象パス。
        path: String,
        /// 元となった I/O エラー。
        #[source]
        source: std::io::Error,
    },
    /// UTF-8 でも `Shift_JIS` でも文字列として解釈できない。
    #[error("文字コードを解釈できません(UTF-8 / `Shift_JIS` いずれも不可): {path}")]
    Encoding {
        /// 対象パス。
        path: String,
    },
    /// ヘッダー行が読み取れない(存在しない場合を含む)。
    #[error("ヘッダーの読み取りに失敗: {path}: {reason}")]
    Header {
        /// 対象パス。
        path: String,
        /// 読み取れなかった理由。
        reason: String,
    },
    /// Excel ブックが開けない・解釈できない。
    #[error("Excel ブックを開けません: {path}: {reason}")]
    Workbook {
        /// 対象パス。
        path: String,
        /// 開けなかった理由。
        reason: String,
    },
    /// Excel シートが読み取れない(存在しない場合を含む)。
    #[error("シートの読み取りに失敗: {sheet}({path}): {reason}")]
    Sheet {
        /// 対象パス。
        path: String,
        /// 対象シート名。
        sheet: String,
        /// 読み取れなかった理由。
        reason: String,
    },
    /// アダプタが必須と宣言した列が存在しない(列マッピングの破損)。
    #[error("必須列がありません: {column}({path})")]
    MissingColumn {
        /// 対象パス。
        path: String,
        /// 欠落している列名。
        column: String,
    },
}

/// レコード単位のエラー(隔離して継続する)。人手レビュー用に文字列で保持する。
#[derive(Debug, Clone, thiserror::Error)]
#[error("{record}: {reason}")]
pub struct RecordError {
    /// レコードの識別子(`line:5` や `line:5(J001)` 等)。
    pub record: String,
    /// 隔離した理由。
    pub reason: String,
}

impl RecordError {
    /// レコード識別子と理由からエラーを作る。
    #[must_use]
    pub fn new(record: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            record: record.into(),
            reason: reason.into(),
        }
    }
}

/// 変換用 JSON の出力エラー。
#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    /// 出力先への書き込みに失敗した。
    #[error("出力の書き込みに失敗: {0}")]
    Io(#[from] std::io::Error),
    /// JSON へのシリアライズに失敗した(データ起因)。
    #[error("JSON へのシリアライズに失敗: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_include_context() {
        let io = ConvertError::Io {
            path: "a.csv".to_string(),
            source: std::io::Error::other("gone"),
        };
        assert!(io.to_string().contains("a.csv"));

        let encoding = ConvertError::Encoding {
            path: "b.csv".to_string(),
        };
        assert!(encoding.to_string().contains("`Shift_JIS`"));

        let header = ConvertError::Header {
            path: "c.csv".to_string(),
            reason: "壊れている".to_string(),
        };
        assert!(header.to_string().contains("壊れている"));

        let missing = ConvertError::MissingColumn {
            path: "d.csv".to_string(),
            column: "job_no".to_string(),
        };
        assert!(missing.to_string().contains("job_no"));

        let workbook = ConvertError::Workbook {
            path: "e.xlsx".to_string(),
            reason: "zip ではない".to_string(),
        };
        assert!(workbook.to_string().contains("e.xlsx"));

        let sheet = ConvertError::Sheet {
            path: "f.xlsx".to_string(),
            sheet: "求人".to_string(),
            reason: "見つからない".to_string(),
        };
        assert!(sheet.to_string().contains("求人"));

        let record = RecordError::new("line:5", "必須欄が空: title");
        assert_eq!(record.to_string(), "line:5: 必須欄が空: title");

        let output_io = OutputError::from(std::io::Error::other("full"));
        assert!(output_io.to_string().contains("full"));
        let serialize = OutputError::from(serde_json::from_str::<usize>("x").unwrap_err());
        assert!(serialize.to_string().contains("シリアライズ"));
    }
}
