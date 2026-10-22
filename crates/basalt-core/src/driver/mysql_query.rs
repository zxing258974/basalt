//! MySQL / MariaDB 查询标签页的连接:用文本协议(COM_QUERY)执行用户的 SQL。
//!
//! 不用预备语句:CREATE PROCEDURE、LOCK TABLES 这类语句不能预备。文本协议里每个值都是文本
//! (BIT 例外,是原始字节),按结果列的类型转成 [`CellValue`]。

use sqlx::mysql::{MySqlConnection, MySqlPool, MySqlRow};
use sqlx::{Connection, Executor, Row, TypeInfo, ValueRef};

use super::mysql::MysqlSession;
use crate::error::CoreResult;
use crate::query::ResultColumn;
use crate::query::collect::{Collected, collect, described_columns};
use crate::sql::{Dialect, dynamic, quote_ident};
use crate::value::CellValue;

pub(crate) struct MysqlEditor {
    pub(super) conn: MySqlConnection,
    thread_id: u64,
}

impl MysqlSession {
    /// 从池里取一条连接并脱离连接池:它从此只归这个查询标签页用,池会补一条新的。
    pub(crate) async fn open_editor(&self) -> CoreResult<MysqlEditor> {
        let mut conn = self.pool.acquire().await?.detach();
        let row = (&mut conn)
            .fetch_one("SELECT CAST(CONNECTION_ID() AS UNSIGNED)")
            .await?;
        let thread_id = row.try_get(0)?;
        Ok(MysqlEditor { conn, thread_id })
    }

    /// 发 KILL QUERY 用的连接池:被取消的连接自己正忙着,得换一条连接去说。
    pub(crate) fn cancel_pool(&self) -> MySqlPool {
        self.pool.clone()
    }
}

impl MysqlEditor {
    pub(crate) fn thread_id(&self) -> u64 {
        self.thread_id
    }

    pub(crate) async fn use_database(&mut self, database: &str) -> CoreResult<()> {
        let sql = format!("USE {}", quote_ident(Dialect::Mysql, database));
        // 不带参数的 SQL 文本走文本协议,和 raw_sql 一样。
        (&mut self.conn).execute(dynamic(sql)).await?;
        Ok(())
    }

    /// 走文本协议:预备语句里的 DATABASE() 会在预备时就定下来,缓存的语句之后一直返回旧值。
    pub(crate) async fn current_database(&mut self) -> CoreResult<Option<String>> {
        let row = (&mut self.conn).fetch_one("SELECT DATABASE()").await?;
        Ok(row.try_get(0)?)
    }

    pub(crate) async fn run(&mut self, sql: &str) -> Collected {
        let stream = sqlx::raw_sql(dynamic(sql)).fetch_many(&mut self.conn);
        collect(
            stream,
            decode_text_cell,
            sqlx::mysql::MySqlQueryResult::rows_affected,
        )
        .await
    }

    pub(crate) async fn describe(&mut self, sql: &str) -> Vec<ResultColumn> {
        described_columns((&mut self.conn).prepare(dynamic(sql)).await)
    }

    pub(crate) async fn ping(&mut self) -> bool {
        self.conn.ping().await.is_ok()
    }

    pub(crate) async fn clear_cached_statements(&mut self) -> Result<(), sqlx::Error> {
        self.conn.clear_cached_statements().await
    }

    pub(crate) async fn close(self) {
        if let Err(error) = self.conn.close().await {
            log::warn!("关闭查询标签页的 MySQL 连接失败：{error}");
        }
    }
}

/// 在另一条连接上让服务器中止这条连接正在执行的语句,连接本身保留。
pub(crate) async fn kill_query(pool: &MySqlPool, thread_id: u64) -> CoreResult<()> {
    // thread_id 是整数,直接拼进去;KILL 的参数不能用占位符。
    let sql = format!("KILL QUERY {thread_id}");
    pool.execute(dynamic(sql)).await?;
    Ok(())
}

pub(crate) fn decode_text_cell(row: &MySqlRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return CellValue::unsupported("?");
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_owned();
    match row.try_get_unchecked::<Vec<u8>, _>(index) {
        Ok(bytes) => text_value(&type_name, bytes),
        Err(error) => {
            log::warn!("读取 MySQL {type_name} 失败：{error}");
            CellValue::Unsupported { type_name }
        }
    }
}

/// 文本协议里的一个非 NULL 值。数字解析失败(不该发生)时保留原文,不丢数据。
fn text_value(type_name: &str, bytes: Vec<u8>) -> CellValue {
    match type_name {
        "BOOLEAN" | "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => {
            parsed(bytes, CellValue::Int)
        }
        name if name.ends_with(" UNSIGNED") => parsed(bytes, CellValue::UInt),
        "YEAR" => parsed(bytes, CellValue::UInt),
        // BIT 在文本协议里也是大端字节。
        "BIT" if bytes.len() <= 8 => CellValue::UInt(
            bytes
                .iter()
                .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte)),
        ),
        "FLOAT" | "DOUBLE" => parsed(bytes, CellValue::Float),
        "DECIMAL" => text_or_bytes(bytes, CellValue::Decimal),
        "DATE" => text_or_bytes(bytes, CellValue::Date),
        "DATETIME" | "TIMESTAMP" => text_or_bytes(bytes, CellValue::DateTime),
        "TIME" => text_or_bytes(bytes, CellValue::Time),
        "JSON" => text_or_bytes(bytes, CellValue::Json),
        "GEOMETRY" | "BIT" => CellValue::Bytes(bytes),
        // 二进制类型也可能是 utf8mb4_bin 这类排序规则的文本列(线上报成 VARBINARY):
        // 结果集里拿不到列的字符集,是有效 UTF-8 就按文本显示。
        _ => text_or_bytes(bytes, CellValue::Text),
    }
}

fn text_or_bytes(bytes: Vec<u8>, wrap: fn(String) -> CellValue) -> CellValue {
    String::from_utf8(bytes).map_or_else(|e| CellValue::Bytes(e.into_bytes()), wrap)
}

fn parsed<T: std::str::FromStr>(bytes: Vec<u8>, wrap: fn(T) -> CellValue) -> CellValue {
    match std::str::from_utf8(&bytes)
        .ok()
        .and_then(|s| s.parse().ok())
    {
        Some(value) => wrap(value),
        None => text_or_bytes(bytes, CellValue::Text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(type_name: &str, value: &str) -> CellValue {
        text_value(type_name, value.as_bytes().to_vec())
    }

    #[test]
    fn numbers_keep_full_precision() {
        assert_eq!(
            text("BIGINT", "-9223372036854775808"),
            CellValue::Int(i64::MIN)
        );
        assert_eq!(
            text("BIGINT UNSIGNED", "18446744073709551615"),
            CellValue::UInt(u64::MAX)
        );
        assert_eq!(text("BOOLEAN", "1"), CellValue::Int(1));
        assert_eq!(text("YEAR", "2026"), CellValue::UInt(2026));
        assert_eq!(text("FLOAT", "0.1"), CellValue::Float(0.1));
        assert_eq!(
            text("DECIMAL", "12345678901234567890123456789.123"),
            CellValue::Decimal("12345678901234567890123456789.123".into())
        );
        assert_eq!(
            text("INT", "abc"),
            CellValue::Text("abc".into()),
            "解析不了就保留原文"
        );
    }

    #[test]
    fn temporal_values_are_shown_as_sent() {
        assert_eq!(
            text("DATE", "0000-00-00"),
            CellValue::Date("0000-00-00".into())
        );
        assert_eq!(
            text("DATETIME", "2026-10-20 08:05:09.12"),
            CellValue::DateTime("2026-10-20 08:05:09.12".into())
        );
        assert_eq!(
            text("TIME", "-838:59:59"),
            CellValue::Time("-838:59:59".into())
        );
        assert_eq!(
            text("JSON", "{\"a\": 1}"),
            CellValue::Json("{\"a\": 1}".into())
        );
    }

    #[test]
    fn bit_is_big_endian_bytes() {
        assert_eq!(text_value("BIT", vec![0x01, 0x02]), CellValue::UInt(258));
        assert_eq!(text_value("BIT", vec![0; 9]), CellValue::Bytes(vec![0; 9]));
    }

    #[test]
    fn binary_types_show_text_only_when_valid_utf8() {
        assert_eq!(text("VARBINARY", "中文"), CellValue::Text("中文".into()));
        assert_eq!(
            text_value("BLOB", vec![0xff, 0x00]),
            CellValue::Bytes(vec![0xff, 0x00])
        );
        assert_eq!(
            text_value("GEOMETRY", b"abc".to_vec()),
            CellValue::Bytes(b"abc".to_vec())
        );
        assert_eq!(text("VARCHAR", "x"), CellValue::Text("x".into()));
        assert_eq!(text_value("TEXT", vec![0xc3]), CellValue::Bytes(vec![0xc3]));
    }
}
