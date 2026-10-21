use super::*;

#[tokio::test]
async fn cancel_is_remembered_while_preparing_and_ignored_when_idle() {
    let slot = EditorSlot::new("c1");
    assert!(!slot.request_cancel().await.unwrap(), "什么都没在做");
    *slot.phase.lock().await = Phase::Preparing {
        cancel_requested: false,
    };
    assert!(slot.request_cancel().await.unwrap());
    assert!(matches!(
        *slot.phase.lock().await,
        Phase::Preparing {
            cancel_requested: true
        }
    ));
}

#[test]
fn statements_cancelled_before_start_report_it() {
    let outcome = cancelled_before_start(QueryScope::default(), Some("重连了".into()));
    assert_eq!(outcome.status, OutcomeStatus::Cancelled);
    assert!(outcome.result_sets.is_empty());
    assert_eq!(outcome.notice.as_deref(), Some("重连了"));
}
