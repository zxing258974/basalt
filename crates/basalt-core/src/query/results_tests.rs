use super::*;

fn text(s: &str) -> CellValue {
    CellValue::Text(s.into())
}

fn store_with(rows: Vec<Vec<CellValue>>) -> (ResultStore, u64) {
    let mut store = ResultStore::default();
    let columns = rows.first().map_or(1, Vec::len);
    let (id, _) = store.add(columns, rows, 0);
    (store, id)
}

fn column(rows: &[Vec<CellValue>], index: usize) -> Vec<CellValue> {
    rows.iter().map(|row| row[index].clone()).collect()
}

#[tokio::test]
async fn rows_come_back_in_pages() {
    let rows: Vec<Vec<CellValue>> = (0..25).map(|n| vec![CellValue::Int(n)]).collect();
    let (store, id) = store_with(rows);
    let stored = store.get(id).unwrap();
    let page = stored.clone().rows(10, 10, None).await.unwrap();
    assert_eq!(
        column(&page, 0),
        (10..20).map(CellValue::Int).collect::<Vec<_>>()
    );
    assert_eq!(stored.clone().rows(20, 10, None).await.unwrap().len(), 5);
    assert!(stored.clone().rows(99, 10, None).await.unwrap().is_empty());
    assert_eq!(stored.rows(0, usize::MAX, None).await.unwrap().len(), 25);
}

#[tokio::test]
async fn a_batch_is_capped() {
    let rows: Vec<Vec<CellValue>> = (0..MAX_BATCH as i64 + 10)
        .map(|n| vec![CellValue::Int(n)])
        .collect();
    let (store, id) = store_with(rows);
    let page = store.get(id).unwrap().rows(0, usize::MAX, None).await;
    assert_eq!(page.unwrap().len(), MAX_BATCH);
}

#[tokio::test]
async fn sorting_puts_nulls_first_numbers_by_value_and_chinese_by_pinyin() {
    let rows = vec![
        vec![text("天津"), CellValue::Int(10)],
        vec![text("北京"), CellValue::Decimal("9.5".into())],
        vec![CellValue::Null, CellValue::Int(-1)],
        vec![text("上海"), CellValue::Null],
        vec![text("a10"), CellValue::Float(2.0)],
        vec![text("a2"), CellValue::UInt(3)],
    ];
    let (store, id) = store_with(rows);
    let stored = store.get(id).unwrap();
    let sort = |column, descending| Some(ResultSort { column, descending });

    let by_name = stored.clone().rows(0, 10, sort(0, false)).await.unwrap();
    assert_eq!(
        column(&by_name, 0),
        // 和前端的 localeCompare("zh-CN") 一样:汉字排在拉丁字母前面。
        [
            CellValue::Null,
            text("北京"),
            text("上海"),
            text("天津"),
            text("a2"),
            text("a10"),
        ]
    );
    let by_number = stored.clone().rows(0, 10, sort(1, true)).await.unwrap();
    assert_eq!(
        column(&by_number, 1),
        [
            CellValue::Int(10),
            CellValue::Decimal("9.5".into()),
            CellValue::UInt(3),
            CellValue::Float(2.0),
            CellValue::Int(-1),
            CellValue::Null,
        ]
    );
    // 接着往下翻用的是同一个排序。
    let rest = stored.rows(4, 10, sort(1, true)).await.unwrap();
    assert_eq!(column(&rest, 1), [CellValue::Int(-1), CellValue::Null]);
}

#[tokio::test]
async fn equal_values_keep_their_order_both_ways() {
    let rows = vec![
        vec![CellValue::Int(1), text("甲")],
        vec![CellValue::Int(0), text("乙")],
        vec![CellValue::Int(1), text("丙")],
    ];
    let (store, id) = store_with(rows);
    let stored = store.get(id).unwrap();
    for descending in [false, true] {
        let sorted = stored
            .clone()
            .rows(
                0,
                10,
                Some(ResultSort {
                    column: 0,
                    descending,
                }),
            )
            .await
            .unwrap();
        let ones: Vec<_> = sorted
            .iter()
            .filter(|row| row[0] == CellValue::Int(1))
            .map(|row| row[1].clone())
            .collect();
        assert_eq!(ones, [text("甲"), text("丙")]);
    }
}

#[tokio::test]
async fn sorting_by_a_missing_column_is_an_error() {
    let (store, id) = store_with(vec![vec![CellValue::Int(1)]]);
    let sort = Some(ResultSort {
        column: 3,
        descending: false,
    });
    let error = store.get(id).unwrap().rows(0, 10, sort).await.unwrap_err();
    assert!(error.to_string().contains("第 4 列"), "{error}");
}

#[test]
fn released_and_old_results_are_gone() {
    let mut store = ResultStore::default();
    let (first, _) = store.add(1, vec![vec![CellValue::Int(1)]], 0);
    let (second, _) = store.add(1, Vec::new(), 0);
    assert!(second > first);
    assert_eq!(store.release(&[first, 999]).len(), 1);
    assert_eq!(store.get(first).err().unwrap().to_string(), RELEASED);
    assert!(store.get(second).is_ok());

    let evicted: usize = (0..MAX_KEPT)
        .map(|_| store.add(1, Vec::new(), 0).1.len())
        .sum();
    assert_eq!(evicted, 1);
    assert_eq!(store.len(), MAX_KEPT);
    assert!(store.get(second).is_err(), "最早的被挤掉了");
}

#[test]
fn row_size_counts_text_and_bytes() {
    let small = row_bytes(&[CellValue::Int(1)]);
    let large = row_bytes(&[CellValue::Text("x".repeat(1000))]);
    assert_eq!(large - small, 1000);
    assert_eq!(row_bytes(&[CellValue::Bytes(vec![0; 10])]) - small, 10);
}

#[test]
fn mixed_numbers_and_text_still_sort_consistently() {
    // SQLite 一列里可以混着放;任何三个值都不能比出 a<b<c<a。
    let values = [
        CellValue::Float(0.25),
        CellValue::Float(0.5),
        text("0.7"),
        CellValue::Float(f64::INFINITY),
        CellValue::Float(f64::NAN),
        CellValue::Int(3),
        CellValue::Decimal("3".into()),
        CellValue::Null,
        text("abc"),
    ];
    for a in &values {
        for b in &values {
            assert_eq!(
                compare_cells(a, b),
                compare_cells(b, a).reverse(),
                "{a:?} {b:?}"
            );
            for c in &values {
                if compare_cells(a, b).is_le() && compare_cells(b, c).is_le() {
                    assert!(compare_cells(a, c).is_le(), "{a:?} {b:?} {c:?}");
                }
            }
        }
    }
    assert!(compare_cells(&CellValue::Float(f64::INFINITY), &CellValue::Int(5)).is_gt());
}

#[test]
fn ids_are_unique_across_stores() {
    let mut one = ResultStore::default();
    let mut two = ResultStore::default();
    let (a, _) = one.add(1, Vec::new(), 0);
    let (b, _) = two.add(1, Vec::new(), 0);
    assert_ne!(a, b, "断开重连后的新存储也不能和旧的撞号");
    assert!(budget() <= MEMORY_BUDGET);
}
