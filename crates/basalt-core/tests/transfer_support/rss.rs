//! 量本进程的常驻内存:导出时内存不能随行数涨。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// 后台每 50 毫秒量一次本进程的常驻内存(KB),记下最大值。
pub struct RssProbe {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub fn rss_kb() -> u64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

impl RssProbe {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(rss_kb()));
        let (s, p) = (stop.clone(), peak.clone());
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                p.fetch_max(rss_kb(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        Self {
            stop,
            peak,
            thread: Some(thread),
        }
    }

    pub fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
        self.peak.load(Ordering::Relaxed).max(rss_kb())
    }
}
