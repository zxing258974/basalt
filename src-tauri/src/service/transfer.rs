//! 导入导出:逻辑在 core 的 `transfer` 里,这里管正在进行的任务(取消要找得到它)。
//!
//! 每个任务由前端起一个 id:导出、导入开始前前端就拿着它,命令还没返回时也能取消。
//! 取消先亮任务自己的开关(借到连接之前也算数),再走查询标签页同一套取消(KILL QUERY /
//! pg_cancel_backend / SQLite 中断)。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use basalt_core::CoreError;
use basalt_core::table::TableRef;
use basalt_core::transfer::export::{
    ExportOutcome, ExportRequest, ExportSource, run_export, transfer_editor_id,
};
use basalt_core::transfer::import::{
    ImportPreview, ImportReport, ImportRequest, preview_import, run_import,
};
use basalt_core::transfer::progress::{CancelFlag, ProgressSink};
use basalt_core::transfer::report::{ImportFailure, write_failures_csv};
use basalt_core::transfer::source::SourceSpec;

use super::CommandResult;
use crate::state::AppState;

struct Job {
    cancel: CancelFlag,
    /// 取消时要中止哪条专用连接上的语句。
    editor_id: String,
}

/// 正在进行的导入导出。
#[derive(Default)]
pub struct TransferJobs {
    jobs: Mutex<HashMap<String, Job>>,
}

impl TransferJobs {
    fn jobs(&self) -> std::sync::MutexGuard<'_, HashMap<String, Job>> {
        self.jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn start(&self, job_id: &str, editor_id: String) -> CommandResult<CancelFlag> {
        let mut jobs = self.jobs();
        if jobs.contains_key(job_id) {
            return Err(CoreError::AlreadyExists(format!("任务 {job_id} 已经在进行中。")).into());
        }
        let cancel = CancelFlag::default();
        jobs.insert(
            job_id.to_owned(),
            Job {
                cancel: cancel.clone(),
                editor_id,
            },
        );
        Ok(cancel)
    }

    fn finish(&self, job_id: &str) {
        self.jobs().remove(job_id);
    }

    /// 亮起任务的取消开关,返回它在用的专用连接;没有这个任务时返回 `None`。
    fn cancel(&self, job_id: &str) -> Option<String> {
        self.jobs().get(job_id).map(|job| {
            job.cancel.cancel();
            job.editor_id.clone()
        })
    }
}

pub async fn export_data(
    state: &AppState,
    job_id: &str,
    request: &ExportRequest,
    progress: &dyn ProgressSink,
) -> CommandResult<ExportOutcome> {
    let editor_id = match &request.source {
        ExportSource::Query { editor_id, .. } => editor_id.clone(),
        _ => transfer_editor_id(job_id),
    };
    let cancel = state.transfers.start(job_id, editor_id)?;
    let result = run_export(&state.sessions, job_id, request, &cancel, progress).await;
    state.transfers.finish(job_id);
    Ok(result?)
}

pub async fn preview_import_file(
    state: &AppState,
    connection_id: &str,
    table: &TableRef,
    source: SourceSpec,
) -> CommandResult<ImportPreview> {
    Ok(preview_import(&state.sessions, connection_id, table, source).await?)
}

pub async fn import_data(
    state: &AppState,
    job_id: &str,
    request: &ImportRequest,
    progress: &dyn ProgressSink,
) -> CommandResult<ImportReport> {
    let cancel = state.transfers.start(job_id, transfer_editor_id(job_id))?;
    let result = run_import(&state.sessions, job_id, request, &cancel, progress).await;
    state.transfers.finish(job_id);
    Ok(result?)
}

/// 取消正在进行的导入导出。任务已经结束(或者没有这个任务)时返回 false。
pub async fn cancel_transfer(state: &AppState, job_id: &str) -> CommandResult<bool> {
    let Some(editor_id) = state.transfers.cancel(job_id) else {
        return Ok(false);
    };
    state.sessions.cancel_query(&editor_id).await?;
    Ok(true)
}

pub fn save_import_failures(path: &str, failures: &[ImportFailure]) -> CommandResult<u64> {
    Ok(write_failures_csv(Path::new(path), failures)?)
}

/// 在系统的文件管理器里选中导出的文件:macOS 是访达,Windows 是资源管理器。
pub fn reveal_in_finder(path: &str) -> CommandResult<()> {
    let path = Path::new(path);
    if !path.is_absolute() || !path.exists() {
        return Err(CoreError::NotFound(format!("找不到文件：{}", path.display())).into());
    }
    let mut command = reveal_command(path);
    let status = command
        .status()
        .map_err(|error| CoreError::File(error.to_string()))?;
    // explorer.exe 选中文件后退出码也是 1,不能当失败。
    if status.success() || cfg!(windows) {
        Ok(())
    } else {
        Err(CoreError::File("没能在访达里打开".into()).into())
    }
}

#[cfg(target_os = "macos")]
fn reveal_command(path: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg("-R").arg(path);
    command
}

#[cfg(windows)]
fn reveal_command(path: &Path) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    // 写成 /select,"路径" 原样传过去:用 arg() 的话,路径带空格时整个参数会被加上引号,
    // explorer 认不出被引起来的开关,只会打开默认文件夹。Windows 路径里不会有双引号。
    let mut argument = std::ffi::OsString::from("/select,\"");
    argument.push(path.as_os_str());
    argument.push("\"");
    let mut command = std::process::Command::new("explorer.exe");
    command.raw_arg(argument);
    command
}

#[cfg(not(any(target_os = "macos", windows)))]
fn reveal_command(path: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path.parent().unwrap_or(path));
    command
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
