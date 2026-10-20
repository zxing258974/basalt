//! 执行历史:每执行一条语句往文件末尾追加一行 JSON(JSON Lines),只留最近的若干条。
//! 写到一半崩溃留下的半行、认不出的行读的时候跳过,不影响其余的。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{Local, SecondsFormat};
use serde::{Deserialize, Serialize};

use super::{OutcomeStatus, QueryScope, StatementOutcome};
use crate::error::{CoreError, CoreResult};

/// 默认保留的条数。
pub const MAX_ENTRIES: usize = 2000;
/// 超出这么多条才整理一次文件,免得每次追加都重写。
const COMPACT_SLACK: usize = 200;
/// 很长的 SQL(比如粘贴进来的整份转储)只记前这么多个字符。
const MAX_SQL_CHARS: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    /// RFC 3339,本机时区。
    pub executed_at: String,
    pub connection_id: String,
    pub connection_name: String,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub schema: Option<String>,
    pub sql: String,
    pub status: OutcomeStatus,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub affected_rows: Option<u64>,
    /// 各结果集带回的行数之和;没有结果集为 `None`。
    #[serde(default)]
    pub row_count: Option<u64>,
}

impl HistoryEntry {
    /// 按一次执行的结果生成一条历史,时间取现在。`scope` 是执行时标签页所在的库。
    pub fn record(
        connection_id: &str,
        connection_name: &str,
        scope: &QueryScope,
        sql: &str,
        outcome: &StatementOutcome,
    ) -> Self {
        let rows: usize = outcome.result_sets.iter().map(|set| set.rows.len()).sum();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            executed_at: Local::now().to_rfc3339_opts(SecondsFormat::Secs, false),
            connection_id: connection_id.to_owned(),
            connection_name: connection_name.to_owned(),
            database: scope.database.clone(),
            schema: scope.schema.clone(),
            sql: sql.chars().take(MAX_SQL_CHARS).collect(),
            status: outcome.status,
            elapsed_ms: outcome.elapsed_ms,
            affected_rows: outcome.affected_rows,
            row_count: (!outcome.result_sets.is_empty()).then_some(rows as u64),
        }
    }

    fn matches(&self, terms: &[String], connection_id: Option<&str>) -> bool {
        if connection_id.is_some_and(|id| id != self.connection_id) {
            return false;
        }
        let haystack = format!(
            "{}\n{}\n{}",
            self.sql,
            self.connection_name,
            self.database.as_deref().unwrap_or_default()
        )
        .to_lowercase();
        terms.iter().all(|term| haystack.contains(term.as_str()))
    }
}

pub struct HistoryStore {
    path: PathBuf,
    capacity: usize,
    /// 文件里的行数:第一次追加时数一遍,之后自己记。同时串行化读写。
    lines: std::sync::Mutex<Option<usize>>,
}

impl HistoryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::with_capacity(path, MAX_ENTRIES)
    }

    pub fn with_capacity(path: impl Into<PathBuf>, capacity: usize) -> Self {
        Self {
            path: path.into(),
            capacity,
            lines: std::sync::Mutex::new(None),
        }
    }

    pub fn append(&self, entry: &HistoryEntry) -> CoreResult<()> {
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| storage_error(dir, e))?;
        }
        let line = serde_json::to_string(entry).map_err(|e| storage_error(&self.path, e))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| storage_error(&self.path, e))?;
        writeln!(file, "{line}").map_err(|e| storage_error(&self.path, e))?;
        let count = match *lines {
            Some(count) => count + 1,
            None => self.read_lines()?.len(),
        };
        *lines = Some(if count > self.capacity + COMPACT_SLACK {
            self.compact()?
        } else {
            count
        });
        Ok(())
    }

    /// 按空白分开的每个词都要出现在 SQL、连接名或库名里(不分大小写)。最新的在前。
    pub fn search(
        &self,
        query: &str,
        connection_id: Option<&str>,
        limit: usize,
    ) -> CoreResult<Vec<HistoryEntry>> {
        let _guard = self
            .lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        Ok(self
            .read_entries()?
            .into_iter()
            .rev()
            .filter(|entry| entry.matches(&terms, connection_id))
            .take(limit)
            .collect())
    }

    pub fn clear(&self) -> CoreResult<()> {
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(storage_error(&self.path, e)),
        }
        *lines = Some(0);
        Ok(())
    }

    fn read_lines(&self) -> CoreResult<Vec<String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_owned)
                .collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(storage_error(&self.path, e)),
        }
    }

    fn read_entries(&self) -> CoreResult<Vec<HistoryEntry>> {
        Ok(self
            .read_lines()?
            .iter()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect())
    }

    /// 只留最近的 `capacity` 条,先写临时文件再改名。返回留下的行数。
    fn compact(&self) -> CoreResult<usize> {
        let lines = self.read_lines()?;
        let keep = &lines[lines.len().saturating_sub(self.capacity)..];
        let tmp = self.path.with_extension("jsonl.tmp");
        let mut text = keep.join("\n");
        text.push('\n');
        std::fs::write(&tmp, text).map_err(|e| storage_error(&tmp, e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| storage_error(&self.path, e))?;
        Ok(keep.len())
    }
}

fn storage_error(path: &Path, error: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("{}：{error}", path.display()))
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
