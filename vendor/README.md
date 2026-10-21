# vendor/

打了补丁的第三方 crate,经根目录 `Cargo.toml` 的 `[patch.crates-io]` 替换 crates.io 上的同名版本。
不是工作区成员(`exclude = ["vendor"]`),`cargo fmt` / `clippy` 不管它们。

## sqlx-core 0.8.6

crates.io 上的 sqlx-core 0.8.6 原样复制过来,只改了这些:

| 位置 | 改动 |
|---|---|
| `src/net/tls/util.rs` | `StdSocket::poll_ready` 换成 sqlx 0.9.0 的实现,并加了单元测试 |
| `Cargo.toml` | 去掉 `[dev-dependencies]`(sqlx 自身、tokio,只给文档测试用),加 `[lints.rust]` 关掉一条新版 rustc 对原代码的提示 |

**为什么改**:sqlx 0.8 的 TLS 套接字(rustls)在一次 I/O 读写都被阻塞时,`poll_ready` 先等可写、再等可读。
rustls 0.23 的 `complete_io` 在写被阻塞(WouldBlock)后还会试着读一次,于是两个方向都记成「在等」;
这时可写先到,就转去只等可读——而服务器正在等这条请求剩下的部分,什么也不会发,两边互等,永远卡住。
套接字的发送、接收队列都是空的,服务器那边这条连接显示 Sleep(MySQL / MariaDB)。

要碰上得同时满足:连接开了 TLS(MySQL 8.x、MariaDB 11.4 默认就开,PostgreSQL 看服务器配置)、
一次写的包比套接字发送缓冲大(导入时一批 INSERT 约 1 MB),再加上「可写」恰好在两次检查之间到达。
最初是在全量测试里碰上的;本机只连 MariaDB 反复跑 transfer_large,四十来遍里又卡住过一次,机器负载越高越容易碰上;
只在 MariaDB 上反复导入、取消的压力测试第 4 轮就卡住了。

sqlx 0.9.0 已经修了(任一方向就绪就返回,让调用方重试),0.8 没有带这个修复的补丁版本;
0.9 要求 Rust 1.94,而且是不兼容升级,所以先用这份副本。

**测试**:

```bash
cargo test -p sqlx-core --lib net::tls::util
```

去掉补丁(换回原来的 `poll_ready`)时 `writable_is_enough_when_both_directions_blocked`、
`readable_is_enough_when_both_directions_blocked` 两条会失败。

**什么时候删**:升级到 sqlx 0.9 以后,删掉整个 `vendor/sqlx-core` 和根目录 `Cargo.toml` 末尾的 `[patch.crates-io]`。
