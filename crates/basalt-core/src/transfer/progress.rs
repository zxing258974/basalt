//! 导入导出的进度与取消。进度经调用方给的 [`ProgressSink`] 报出去(Tauri 外壳把它转成事件),
//! 节流到最多每 100 毫秒一次,阶段变了立即报。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

const INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// 连接、读表结构、打开文件。
    Preparing,
    Exporting,
    /// 导入前先把整份文件按目标列检查一遍(遇错即停时)。
    Checking,
    Importing,
    /// 写文件尾、提交事务。
    Finishing,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    /// 已经处理的行数。
    pub rows: u64,
    /// 导入时失败的行数。
    pub failed: u64,
    /// 已知总量时的完成比例(0–1)。
    pub fraction: Option<f64>,
}

impl Progress {
    pub fn new(phase: Phase, rows: u64) -> Self {
        Self {
            phase,
            rows,
            failed: 0,
            fraction: None,
        }
    }
}

pub trait ProgressSink: Send + Sync {
    fn report(&self, progress: &Progress);
}

impl<F: Fn(&Progress) + Send + Sync> ProgressSink for F {
    fn report(&self, progress: &Progress) {
        self(progress);
    }
}

/// 什么都不报,测试与不关心进度的调用方用。
pub fn ignore(_: &Progress) {}

pub struct Throttle<'a> {
    sink: &'a dyn ProgressSink,
    last: Option<(Instant, Phase)>,
}

impl<'a> Throttle<'a> {
    pub fn new(sink: &'a dyn ProgressSink) -> Self {
        Self { sink, last: None }
    }

    /// 距上次不到 100 毫秒、阶段也没变就不报。
    pub fn report(&mut self, progress: &Progress) {
        let now = Instant::now();
        let due = match self.last {
            Some((at, phase)) => phase != progress.phase || now.duration_since(at) >= INTERVAL,
            None => true,
        };
        if due {
            self.force(progress);
        }
    }

    pub fn force(&mut self, progress: &Progress) {
        self.last = Some((Instant::now(), progress.phase));
        self.sink.report(progress);
    }
}

/// 调用方持有的取消开关:取消请求可能在借到连接之前就到了,光靠连接上的取消会漏掉。
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;

    #[test]
    fn throttle_drops_rapid_updates_but_not_phase_changes() {
        let seen = Mutex::new(Vec::new());
        let sink = |p: &Progress| seen.lock().unwrap().push((p.phase, p.rows));
        let mut throttle = Throttle::new(&sink);
        throttle.report(&Progress::new(Phase::Exporting, 1));
        throttle.report(&Progress::new(Phase::Exporting, 2));
        throttle.report(&Progress::new(Phase::Finishing, 3));
        throttle.force(&Progress::new(Phase::Finishing, 4));
        assert_eq!(
            *seen.lock().unwrap(),
            [
                (Phase::Exporting, 1),
                (Phase::Finishing, 3),
                (Phase::Finishing, 4)
            ]
        );
        ignore(&Progress::new(Phase::Preparing, 0));
    }

    #[test]
    fn progress_serializes_in_camel_case_and_flag_is_shared() {
        assert_eq!(
            serde_json::to_value(Progress::new(Phase::Checking, 5)).unwrap(),
            json!({ "phase": "checking", "rows": 5, "failed": 0, "fraction": null })
        );
        let flag = CancelFlag::default();
        let copy = flag.clone();
        copy.cancel();
        assert!(flag.is_cancelled());
    }
}
