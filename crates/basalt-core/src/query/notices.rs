//! 服务器在执行过程中发来的消息:PostgreSQL 的 NOTICE、WARNING(`RAISE NOTICE` 之类)。
//!
//! sqlx 不把它们交给调用方,只当作 target 为 `sqlx::postgres::notice` 的 tracing 事件发出来。
//! 进程里装一个全局的订阅者,只接这类事件,收进当时正在执行的那条语句的列表(task-local);
//! 不在执行语句时(比如连接池上改数据触发了 RAISE NOTICE)丢掉。别的 tracing 事件不接,
//! 照样经 tracing 的 `log-always` 转给 log。
//!
//! 不能只在执行语句时临时挂订阅者:tracing 第一次遇到某个调用点时如果不在执行语句,
//! 会把它记成「没人要」并缓存下来,之后执行语句时也收不到了。

use std::cell::RefCell;
use std::sync::LazyLock;

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::instrument::WithSubscriber;
use tracing::subscriber::Interest;
use tracing::{Dispatch, Event, Level, Metadata, Subscriber, span};

const TARGET: &str = "sqlx::postgres::notice";

/// 服务器发来的一条消息。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerMessage {
    /// NOTICE、WARNING、INFO、DEBUG。
    pub level: String,
    pub text: String,
}

tokio::task_local! {
    static COLLECTED: RefCell<Vec<ServerMessage>>;
}

/// 第一次执行语句时装上。装不上(进程里已经有别的全局订阅者)时退回到执行时临时挂,
/// 那样在上面说的情况下会漏消息,但不至于完全没有。
static GLOBAL: LazyLock<bool> = LazyLock::new(|| {
    let installed =
        tracing::dispatcher::set_global_default(Dispatch::new(NoticeSubscriber)).is_ok();
    if !installed {
        log::warn!("已经有全局的 tracing 订阅者,PostgreSQL 的 NOTICE 只在执行语句时临时收");
    }
    installed
});
static SCOPED: LazyLock<Dispatch> = LazyLock::new(|| Dispatch::new(NoticeSubscriber));

/// 执行 `future`,顺便收下这期间服务器发来的消息。
pub(crate) async fn collecting<F: Future>(future: F) -> (F::Output, Vec<ServerMessage>) {
    let global = *GLOBAL;
    COLLECTED
        .scope(RefCell::new(Vec::new()), async move {
            let output = if global {
                future.await
            } else {
                future.with_subscriber(SCOPED.clone()).await
            };
            (output, COLLECTED.with(RefCell::take))
        })
        .await
}

/// sqlx 按 PostgreSQL 的级别换成 tracing 的级别(NOTICE → INFO、WARNING → WARN、INFO / LOG → TRACE),这里换回去。
fn level_name(level: Level) -> &'static str {
    match level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARNING",
        Level::INFO => "NOTICE",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "INFO",
    }
}

struct NoticeSubscriber;

#[derive(Default)]
struct MessageField(Option<String>);

impl Visit for MessageField {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" && self.0.is_none() {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl Subscriber for NoticeSubscriber {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        // sometimes:临时挂的时候,不在执行语句时由当时的订阅者决定。
        if metadata.target() == TARGET {
            Interest::sometimes()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == TARGET
    }

    fn event(&self, event: &Event<'_>) {
        let mut message = MessageField::default();
        event.record(&mut message);
        let Some(text) = message.0 else {
            return;
        };
        let level = level_name(*event.metadata().level()).to_owned();
        // 不在执行语句(连接池上的触发器之类):丢掉。
        let _ = COLLECTED.try_with(|collected| {
            collected.borrow_mut().push(ServerMessage { level, text });
        });
    }

    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

    fn enter(&self, _: &span::Id) {}

    fn exit(&self, _: &span::Id) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn notices_raised_while_running_are_collected_in_order() {
        let ((), messages) = collecting(async {
            tracing::info!(target: "sqlx::postgres::notice", message = "第一条");
            tracing::warn!(target: "sqlx::postgres::notice", message = "小心");
            tracing::info!(target: "something::else", message = "不收");
        })
        .await;
        assert_eq!(
            messages,
            [
                ServerMessage {
                    level: "NOTICE".into(),
                    text: "第一条".into()
                },
                ServerMessage {
                    level: "WARNING".into(),
                    text: "小心".into()
                },
            ]
        );
        // 不在收集期间发的不会留下来,也不会出错。
        tracing::info!(target: "sqlx::postgres::notice", message = "没人收");
        let ((), none) = collecting(async {}).await;
        assert!(none.is_empty());
    }

    /// 同一个调用点:先在执行语句之外发一次,之后执行语句时发的照样收得到。
    fn raise(text: &str) {
        tracing::info!(target: "sqlx::postgres::notice", message = text);
    }

    #[tokio::test]
    async fn a_notice_seen_outside_a_statement_does_not_silence_later_ones() {
        raise("连接池上的触发器发的");
        let ((), messages) = collecting(async { raise("语句里发的") }).await;
        assert_eq!(
            messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            ["语句里发的"]
        );
    }
}
