//! 上次开着的标签页(查询里写到一半的 SQL 也在内),下次启动时照原样打开。
//!
//! 内容由前端决定、这里只当一份 JSON 文档存取:后端不需要懂标签页长什么样。
//! 写入先写同目录临时文件再改名,写到一半退出也不会留下半个文件;
//! 读到坏文件时把它挪到一边(不删、不覆盖),当作没有上次的记录。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{CoreError, CoreResult};

/// 一份记录最多这么大。查询里贴了整段数据也够用,又不至于把磁盘写满。
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

pub struct WorkspaceStore {
    path: PathBuf,
    /// 两次保存不同时写同一个临时文件。
    lock: Mutex<()>,
}

impl WorkspaceStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    /// 没有记录(第一次启动)返回 None;文件坏了也返回 None,原文件改名留着。
    pub fn load(&self) -> CoreResult<Option<String>> {
        let _guard = self.lock()?;
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                self.set_aside("不是 UTF-8 文本");
                return Ok(None);
            }
            Err(error) => return Err(storage_error(&self.path, error)),
        };
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) if value.is_object() => Ok(Some(text)),
            Ok(_) => {
                self.set_aside("不是 JSON 对象");
                Ok(None)
            }
            Err(error) => {
                self.set_aside(&error.to_string());
                Ok(None)
            }
        }
    }

    pub fn save(&self, content: &str) -> CoreResult<()> {
        if content.len() > MAX_BYTES {
            return Err(CoreError::Storage(format!(
                "打开的标签页内容太多（{} MB），没能记下来：最多 {} MB。可以把大段 SQL 存成查询再关掉标签页",
                content.len() / 1024 / 1024,
                MAX_BYTES / 1024 / 1024
            )));
        }
        match serde_json::from_str::<serde_json::Value>(content) {
            Ok(value) if value.is_object() => {}
            _ => return Err(CoreError::Storage("要记下的标签页不是 JSON 对象".into())),
        }
        let _guard = self.lock()?;
        let dir = self
            .path
            .parent()
            .ok_or_else(|| CoreError::Storage(format!("无效的路径：{}", self.path.display())))?;
        std::fs::create_dir_all(dir).map_err(|e| storage_error(dir, e))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, content).map_err(|e| storage_error(&tmp, e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| storage_error(&self.path, e))
    }

    fn lock(&self) -> CoreResult<std::sync::MutexGuard<'_, ()>> {
        self.lock
            .lock()
            .map_err(|_| CoreError::Storage("标签页记录的锁已损坏".into()))
    }

    /// 读不懂的记录改名成 workspace.json.broken-<时间>,留着以防里面有要找回的 SQL。
    fn set_aside(&self, reason: &str) {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let target = self.path.with_extension(format!("json.broken-{stamp}"));
        match std::fs::rename(&self.path, &target) {
            Ok(()) => log::warn!(
                "上次的标签页记录读不懂（{reason}），已挪到 {}",
                target.display()
            ),
            Err(error) => log::warn!("上次的标签页记录读不懂（{reason}），也没能挪开：{error}"),
        }
    }
}

fn storage_error(path: &Path, error: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("{}：{error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, WorkspaceStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(dir.path().join("config/workspace.json"));
        (dir, store)
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir.join("config"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn first_start_has_nothing_to_restore() {
        let (_dir, store) = store();
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn saves_and_reads_back_without_leaving_temp_files() {
        let (dir, store) = store();
        let content = r#"{"version":1,"tabs":[{"kind":"query","sql":"SELECT '中文';"}]}"#;
        store.save(content).unwrap();
        store.save(content).unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some(content));
        assert_eq!(files(dir.path()), ["workspace.json"]);
    }

    #[test]
    fn refuses_what_is_not_a_json_object_or_too_big() {
        let (_dir, store) = store();
        assert!(store.save("[1, 2]").is_err());
        assert!(store.save("{坏了").is_err());
        let huge = format!(r#"{{"sql":"{}"}}"#, "x".repeat(MAX_BYTES));
        let error = store.save(&huge).unwrap_err().to_string();
        assert!(error.contains("最多 32 MB"), "{error}");
        assert_eq!(store.load().unwrap(), None, "拒绝的内容没有落盘");
    }

    #[test]
    fn a_broken_file_is_set_aside_not_overwritten() {
        let (dir, store) = store();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/workspace.json"), "{写到一半").unwrap();
        assert_eq!(store.load().unwrap(), None);
        let names = files(dir.path());
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("workspace.json.broken-"), "{names:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("config").join(&names[0])).unwrap(),
            "{写到一半"
        );

        store.save(r#"{"version":1}"#).unwrap();
        assert_eq!(files(dir.path()).len(), 2, "坏文件还留着");
    }
}
