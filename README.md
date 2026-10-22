# Basalt

macOS 上的桌面数据库客户端，连 MySQL、MariaDB、PostgreSQL 和 SQLite：看表、改数据、写 SQL、改表结构、导入导出、看服务器在忙什么。
基于 Tauri 2、Rust 和 React，配置和密码只存在本机。

设计取舍、实现细节和已知限制见 [DESIGN.md](DESIGN.md)。以 [MIT](LICENSE) 许可发布。

## 功能

**连接**

- 新建、编辑、复制、删除、分组、测试连接；四种数据库各有表单。
- 连接对话框分「常规 / SSH / SSL」三页：
  - SSH 隧道支持密码、私钥（可带口令）和 SSH Agent 登录，可以串起一台或几台跳板机（写法同 OpenSSH 的 ProxyJump）。
    也能从 `~/.ssh/config` 选一台主机，自动填好地址、用户名、私钥和跳板机。
  - 第一次连某台 SSH 服务器时核对主机密钥指纹，信任后记下来；以后指纹变了会拒绝连接，并说明怎么处理。
  - SSL 有禁用、优先、必须、校验 CA、校验主机名五档，可选 CA 证书和客户端证书。
- 工具栏「导入」：
  - 从 `.ncx` 连接导出文件，或本机另一款数据库客户端的配置里导入连接，隧道和 SSL 设置一起带过来；
  - 也能导入那边保存的查询。
  - 只新增，不覆盖已有的连接和查询。

**浏览与编辑**

- 左边对象树：连接 → 库 → schema → 表 / 视图 / 保存的查询。
- 对象列表页能按名称、注释筛选，按行数、大小排序；右侧信息面板显示表的属性和建表语句。
- 表数据页：
  - 分页、点列头排序、WHERE 筛选；
  - 行内编辑，新增、删除行；
  - 没提交的修改高亮显示，⌘S 先看将要执行的 SQL，再在一个事务里提交。
- 没有主键的表、视图只读。
- 关掉再打开，上次开着的标签页都回来，查询里没保存的 SQL 也在。

**SQL 编辑器**

- 按库结构补全表名和列名。
- 多条语句一条一条执行，每个结果集一个标签；「消息」页写明耗时和影响行数。能取消正在执行的长查询。
- 每个查询标签页有自己的一条连接：`SET @x`、临时表、`BEGIN` 开的事务都留在这个标签页里。
  MySQL 的存储过程不写 `DELIMITER` 也能整体执行。
- 有执行历史；查询能保存成文件，挂在对象树对应库下的「查询」分组里。
- SQL 助手：接 OpenAI 兼容或 Claude 的接口，带上当前库的表结构（不含数据）帮你写 SQL。

**查询结果**

- 结果完整读回来放在后端，表格往下滚自动接着取，点列头按整个结果排序。
- 结果直接取自带主键的表时可以就地改，和表数据页一样预览、提交，提交后重新执行刷新：
  - 单表查询可以增删改行；
  - JOIN 了几张表时一次改一张（底栏选改哪张），只改格子、不增删行；
  - 子查询、UNION 和表达式算出来的列只读，底栏写明原因。
- 导出时按表格上点的排序。

**表结构设计**

- 在字段、索引、外键、选项几页里改，「SQL 预览」页实时显示将要执行的 DDL，⌘S 确认后执行。
- PostgreSQL、SQLite 在一个事务里执行，SQLite 做不到的改动自动重建表；MySQL 逐条执行，中途失败会说明哪些已经生效。

**导入导出**

- 导出成 CSV、Excel、JSON（数组或 JSON Lines）、SQL INSERT。
  - 范围可以是整张表、当前筛选后的结果、查询的全部结果；
  - 边读边写，内存不随行数涨，能中途取消。
- 导入 CSV / Excel 分三步：选文件 → 映射列 → 选项。按目标列的类型解析，解析不了的进失败报告，不悄悄改成 NULL。
- 表节点右键还有：
  - 转储 SQL 文件（结构加数据，能直接执行还原）；
  - 复制表、重命名、维护（分析、检查、优化、VACUUM 等）；
  - 清空、截断、删除。

**服务器监控**

- 连接右键「服务器监控」：
  - 每秒查询数（PostgreSQL 是事务数）、连接数占上限的比例、缓存命中率、收发流量、慢查询，带最近一段的走势；
  - 下面是会话列表，在执行的排前面。
- 能终止某个会话正在执行的语句，或者断开它的连接（先确认，确认后会话变了就不动手）。
- 账号权限不够、只看得到一部分时，页面上写明。

## 快捷键

| 位置 | 按键 | 作用 |
| --- | --- | --- |
| 全局 | ⌘N / ⌘T / ⌘W | 新建连接 / 新建查询 / 关闭标签页 |
| SQL 编辑器 | ⌘↩ | 执行光标所在的语句（有选中文字时执行选中的部分） |
| | ⇧⌘↩ / ⌘. | 全部执行 / 取消 |
| | ⌘S / ⇧⌘S | 保存 / 另存为 |
| | Ctrl-Space | 手动唤出补全 |
| 表格 | 方向键、回车 / 双击 | 移动、编辑 |
| | Tab / Esc | 提交这一格并右移 / 取消 |
| | ⌘C / ⌘⌫ | 复制 / 删除或恢复行 |
| | ⌘S | 预览并提交修改 |

## 安装

从 [Releases](https://github.com/zxing258974/basalt/releases) 下载 Apple 芯片版的 zip，解压后把 `Basalt.app` 拖进「应用程序」。
应用只做了临时签名（ad-hoc），下载的版本第一次打开会被 macOS 拦下，要在「系统设置 → 隐私与安全性」里点「仍要打开」。

也可以从源码构建，生成的 `.app` 在 `target/release/bundle/macos/`：

```bash
pnpm install
pnpm tauri build --bundles app
```

## 数据存在哪里

全部在 `~/Library/Application Support/dev.basalt.app/`：

| 文件 | 内容 |
| --- | --- |
| `connections.json` | 连接配置，不含密码 |
| `secrets.json` | 数据库密码、SSH 密码和私钥口令、SQL 助手的 API Key |
| `known_hosts` | 信任过的 SSH 主机密钥（OpenSSH 格式） |
| `history.jsonl` | 执行历史，最近 2000 条 |
| `queries/<连接 id>/<库>/[<schema>/]<名字>.sql` | 保存的查询；删除连接不会删它的查询 |
| `workspace.json` | 上次开着的标签页 |

`secrets.json` 只有当前用户能读写（权限 600），但**是明文**：备份、网盘同步、本账号下运行的程序都能读到。

SSH 服务器换了主机密钥时，确认无误后删掉 `known_hosts` 里对应的那一行。

早期版本把密码存在钥匙串的 `dev.basalt.app` 服务下：

- 升级后第一次启动时一次搬进 `secrets.json`，钥匙串的授权框都在这时弹完；之后不再碰钥匙串。
- 中途拒绝的话，已经搬过来的留着，剩下的到时重新填。
- 钥匙串里的旧条目不会自动删，可以在「钥匙串访问」里搜 `dev.basalt.app` 手动删。

## 开发

```bash
pnpm install
pnpm tauri dev          # 桌面应用
pnpm dev                # 只起前端，浏览器里自动用 mock 后端，方便调界面
```

```
crates/basalt-core/   核心库（不依赖 Tauri）：连接配置、密码存储、驱动、会话、表数据读写、SQL 生成、
                      SQL 编辑器的执行 / 历史 / 保存的查询、导入导出、服务器监控
src-tauri/            Tauri 外壳：commands.rs 只做转发，逻辑在 service/
src/                  前端：components 按界面区域组织，lib/api.ts 是后端接口，lib/mock* 是浏览器里用的 mock 后端
e2e/                  Playwright 截图测试与基准图
tools/                开发工具
```

ESLint 通过 `tools/typescript-eslint-ts6` 给 typescript-eslint 配了一份 TypeScript 6。
项目本身用的 TypeScript 7 是原生版，没有 typescript-eslint 需要的 JS 编译器接口；`tsc` 仍然用 7。

仓库里留着打 Windows 安装包的工作流（`.github/workflows/windows.yml`），日常开发和测试只在 macOS 上做。

## 测试

第一次跑之前装两样工具（都只装在用户目录）：

```bash
pnpm exec playwright install webkit                                               # 截图测试用的 WebKit
rustup component add llvm-tools-preview && cargo install cargo-llvm-cov --locked  # Rust 覆盖率
```

### 前端

```bash
pnpm lint               # ESLint + Prettier 检查，零告警才算过
pnpm format             # 用 Prettier 格式化
pnpm typecheck
pnpm test               # 单测与交互测试（Vitest + Testing Library，mock 后端）
pnpm test --coverage    # 同上并统计覆盖率，任一项低于 80% 即失败；报告在 coverage/
pnpm test:visual        # 截图回归（Playwright + WebKit，自动在 1430 端口起 pnpm dev）
pnpm test:visual:update # 界面有意改动后重新生成基准图
```

截图测试：

- 关键画面按 960、1280、1440 三种宽度和深浅两套主题各截一张，基准图在 `e2e/__screenshots__/`。
- 只用 WebKit：Tauri 在 macOS 上用的就是它，Chromium 的排版和它有出入。
- 基准图依赖本机字体，换机器或升级系统后先 `pnpm test:visual:update`，再逐张看差异。
- 失败时的对比图在 `test-results/`，`pnpm exec playwright show-report` 可以并排查看。

### Rust

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                      # 单测 + SQLite 集成测试
```

连真实数据库的集成测试要先起测试容器：

```bash
docker compose -f docker-compose.test.yml up -d --build          # MySQL 5.7 + PostgreSQL 17 + sshd
docker compose -f docker-compose.test.yml --profile full up -d   # 再加 MySQL 8.4 + MariaDB 11.4
BASALT_IT=1 cargo test --workspace          # 连 MySQL 5.7、PostgreSQL 与 sshd
BASALT_IT=all cargo test --workspace        # 连全部容器
BASALT_IT=all cargo llvm-cov --workspace --fail-under-lines 80   # 覆盖率
```

容器的端口和账号：

- 容器只监听 127.0.0.1：MySQL 5.7 `23306`、MySQL 8.4 `23307`、MariaDB `23308`、PostgreSQL `25432`、sshd `20022`。
- 账号见 `docker-compose.test.yml`，只用于本机的一次性容器。
- `BASALT_IT=all` 会在测试库里建几个 `basalt_` 开头的库和账号。

SSH 与 SSL 的测试：

- sshd 容器是 SSH 跳板机，和数据库容器在同一个网络，经它连库时地址写容器名（`mysql57`、`postgres`）。
- MySQL 5.7 和 PostgreSQL 挂着 `tests/fixtures/tls` 里测试 CA 签的证书开 SSL。
- `BASALT_IT` 的名单里写 `sshd` 选中跳板机；经隧道的测试要同时选中 sshd 和对应的库。

设了 `BASALT_IT` 时还会跑 `transfer_large`：

- 每种库 10 万行（约 100 MB）导出 CSV / Excel 并量进程内存，中途取消，导回再取消。
- 调试构建下要几分钟，加 `--nocapture` 能看到每一步的用时和内存。
