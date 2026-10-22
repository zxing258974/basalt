//! 查询结果留在后端:执行时把行全部读完存在这里,前端先拿第一批,往下滚再按需取,排序也在这里做。
//! 这样多大的结果都能看全,而 WebView 里只放看过的行。

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use icu_collator::options::CollatorOptions;
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use serde::Deserialize;

use crate::error::{CoreError, CoreResult};
use crate::value::{CellValue, format_float, to_hex};

/// 执行完随结果一起带回前端的行数;其余的往下滚时再取。
pub const FIRST_BATCH: usize = 1000;
/// 一次最多取多少行。
pub const MAX_BATCH: usize = 5000;
/// 一条语句的结果最多存多少(按内存估算)。再多的行照常从服务器读完(连接才能接着用),但不再存。
pub const MEMORY_BUDGET: usize = 1 << 30;
/// 所有标签页存着的结果加起来最多占多少;快满时新结果能存的就少了。
const TOTAL_BUDGET: usize = 2 << 30;
/// 一个标签页最多留多少个结果集:前端没来得及释放的,最早的先丢。
const MAX_KEPT: usize = 64;

/// 结果编号全局递增:标签页的连接断开重连后结果存储是新的,编号也不能和之前的重复,
/// 否则前端放掉旧结果时会把同号的新结果一起放掉。
static NEXT_ID: AtomicU64 = AtomicU64::new(0);
/// 所有存着的结果估算占了多少内存。
static STORED_BYTES: AtomicUsize = AtomicUsize::new(0);

/// 下一条语句的结果还能存多少。
pub(crate) fn budget() -> usize {
    MEMORY_BUDGET.min(TOTAL_BUDGET.saturating_sub(STORED_BYTES.load(AtomicOrdering::Relaxed)))
}

pub(crate) const RELEASED: &str = "这个结果已经释放了，请重新执行。";

/// 按第几列排序(结果里可能有同名列,所以用序号)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultSort {
    pub column: usize,
    pub descending: bool,
}

/// 一行大约占多少内存:够判断有没有超出预算就行。
pub(crate) fn row_bytes(row: &[CellValue]) -> usize {
    size_of::<Vec<CellValue>>()
        + row
            .iter()
            .map(|cell| {
                size_of::<CellValue>()
                    + match cell {
                        CellValue::Decimal(s)
                        | CellValue::Text(s)
                        | CellValue::Json(s)
                        | CellValue::Date(s)
                        | CellValue::Time(s)
                        | CellValue::DateTime(s) => s.len(),
                        CellValue::Bytes(bytes) => bytes.len(),
                        CellValue::Unsupported { type_name } => type_name.len(),
                        CellValue::Null
                        | CellValue::Bool(_)
                        | CellValue::Int(_)
                        | CellValue::UInt(_)
                        | CellValue::Float(_) => 0,
                    }
            })
            .sum::<usize>()
}

pub(crate) struct StoredResult {
    rows: Vec<Vec<CellValue>>,
    columns: usize,
    bytes: usize,
    /// 最近一次排序的行序:往下滚时接着用,不必每批都重排。
    sorted: Mutex<Option<(ResultSort, Arc<Vec<u32>>)>>,
}

impl Drop for StoredResult {
    fn drop(&mut self) {
        STORED_BYTES.fetch_sub(self.bytes, AtomicOrdering::Relaxed);
    }
}

impl StoredResult {
    /// 从第 `offset` 行起取 `limit` 行;给了排序就按排序后的顺序取。大结果排一次可能要几秒,放到阻塞线程上做。
    pub(crate) async fn rows(
        self: Arc<Self>,
        offset: usize,
        limit: usize,
        sort: Option<ResultSort>,
    ) -> CoreResult<Vec<Vec<CellValue>>> {
        let limit = limit.min(MAX_BATCH);
        let Some(sort) = sort else {
            return Ok(self.slice(offset, limit, None));
        };
        if sort.column >= self.columns {
            return Err(CoreError::NotFound(format!(
                "结果里没有第 {} 列",
                sort.column + 1
            )));
        }
        tokio::task::spawn_blocking(move || {
            let order = self.order(sort);
            self.slice(offset, limit, Some(&order))
        })
        .await
        .map_err(|e| CoreError::InvalidData(format!("排序结果失败：{e}")))
    }

    fn slice(&self, offset: usize, limit: usize, order: Option<&[u32]>) -> Vec<Vec<CellValue>> {
        let end = offset.saturating_add(limit).min(self.rows.len());
        let start = offset.min(end);
        match order {
            None => self.rows[start..end].to_vec(),
            Some(order) => order[start..end]
                .iter()
                .map(|&i| self.rows[i as usize].clone())
                .collect(),
        }
    }

    fn order(&self, sort: ResultSort) -> Arc<Vec<u32>> {
        let mut cached = self.sorted.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((key, order)) = cached.as_ref()
            && *key == sort
        {
            return order.clone();
        }
        let mut order: Vec<u32> = (0..self.rows.len() as u32).collect();
        // sort_by 是稳定排序:值相同的行保持原来的先后,倒序时也一样。
        order.sort_by(|&a, &b| {
            let ordering = compare_cells(
                &self.rows[a as usize][sort.column],
                &self.rows[b as usize][sort.column],
            );
            if sort.descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
        let order = Arc::new(order);
        *cached = Some((sort, order.clone()));
        order
    }
}

/// 中文按拼音、字符串里的数字按数值比,和前端 `localeCompare("zh-CN", { numeric: true })` 一样。
static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    let mut prefs = CollatorPreferences::from(icu_locale_core::locale!("zh"));
    prefs.numeric_ordering = Some(CollationNumericOrdering::True);
    Collator::try_new(prefs, CollatorOptions::default()).expect("编进程序里的中文排序数据")
});

/// NULL 排最前,其次是数字(按数值,相等再按文本),最后是其余的值(按文本)。
/// 必须是全序:SQLite 一列里可以数字、文本混着,「数字和文本互相按文本比」会出现 a<b<c<a,排序就乱了。
pub fn compare_cells(a: &CellValue, b: &CellValue) -> Ordering {
    let rank = |cell: &CellValue| match cell {
        CellValue::Null => 0,
        _ if number(cell).is_some() => 1,
        _ => 2,
    };
    rank(a).cmp(&rank(b)).then_with(|| {
        let by_value = match (number(a), number(b)) {
            (Some(x), Some(y)) => x.total_cmp(&y),
            _ => Ordering::Equal,
        };
        by_value.then_with(|| COLLATOR.compare(&text(a), &text(b)))
    })
}

/// 能按数值比的值;NaN 不算(和谁都比不出大小),当文本排。
fn number(cell: &CellValue) -> Option<f64> {
    let value = match cell {
        CellValue::Int(n) => *n as f64,
        CellValue::UInt(n) => *n as f64,
        CellValue::Float(f) => *f,
        CellValue::Decimal(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (!value.is_nan()).then_some(value)
}

/// 和前端显示用的文本一致:数字、布尔转成文本,二进制是十六进制,解不了的类型当空串。
fn text(cell: &CellValue) -> Cow<'_, str> {
    match cell {
        CellValue::Null | CellValue::Unsupported { .. } => Cow::Borrowed(""),
        CellValue::Bool(b) => Cow::Borrowed(if *b { "true" } else { "false" }),
        CellValue::Int(n) => Cow::Owned(n.to_string()),
        CellValue::UInt(n) => Cow::Owned(n.to_string()),
        CellValue::Float(f) => Cow::Owned(format_float(*f)),
        CellValue::Bytes(bytes) => Cow::Owned(to_hex(bytes)),
        CellValue::Decimal(s)
        | CellValue::Text(s)
        | CellValue::Json(s)
        | CellValue::Date(s)
        | CellValue::Time(s)
        | CellValue::DateTime(s) => Cow::Borrowed(s),
    }
}

/// 一个查询标签页存着的结果。
#[derive(Default)]
pub(crate) struct ResultStore {
    sets: BTreeMap<u64, Arc<StoredResult>>,
}

impl ResultStore {
    /// 存下一个结果,返回它的编号和被挤掉的旧结果。旧结果可能很大,交给调用方在放开锁以后再释放。
    pub(crate) fn add(
        &mut self,
        columns: usize,
        rows: Vec<Vec<CellValue>>,
        bytes: usize,
    ) -> (u64, Vec<Arc<StoredResult>>) {
        let id = NEXT_ID.fetch_add(1, AtomicOrdering::Relaxed) + 1;
        STORED_BYTES.fetch_add(bytes, AtomicOrdering::Relaxed);
        self.sets.insert(
            id,
            Arc::new(StoredResult {
                rows,
                columns,
                bytes,
                sorted: Mutex::new(None),
            }),
        );
        let mut evicted = Vec::new();
        while self.sets.len() > MAX_KEPT {
            evicted.extend(self.sets.pop_first().map(|(_, set)| set));
        }
        (id, evicted)
    }

    pub(crate) fn get(&self, id: u64) -> CoreResult<Arc<StoredResult>> {
        self.sets
            .get(&id)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(RELEASED.into()))
    }

    /// 拿走这些结果;同样由调用方在放开锁以后释放。
    pub(crate) fn release(&mut self, ids: &[u64]) -> Vec<Arc<StoredResult>> {
        ids.iter().filter_map(|id| self.sets.remove(id)).collect()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.sets.len()
    }
}

#[cfg(test)]
#[path = "results_tests.rs"]
mod tests;
