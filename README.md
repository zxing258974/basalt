# Basalt

对标 Navicat Premium 使用体验的桌面数据库客户端,自用。Tauri 2 + Rust + React。
设计与里程碑见 [DESIGN.md](DESIGN.md)。

**当前进度:M3**——连接管理(新建 / 编辑 / 复制 / 删除 / 分组 / 测试连接,密码进 macOS 钥匙串)、
MySQL / MariaDB / PostgreSQL / SQLite 四种驱动、对象树(连接 → 库 → schema → 表 / 视图 / 查询)、对象列表页;
数据浏览与编辑:在对象树里双击表、或在对象列表里双击一行打开「表数据」标签页,
分页、点列头排序、WHERE 筛选、行内编辑、新增 / 删除行,未提交的修改高亮,⌘S 先看 SQL 预览再在一个事务里提交;
SQL 编辑器:工具栏「新建查询」(⌘T)或在库 / schema 上右键「新建查询」,按库结构补全表名和列名,
多条语句一条一条执行、每个结果集一个标签、「消息」页写明耗时和影响行数,能取消正在执行的长查询,
有执行历史,查询能保存成文件、挂在对象树对应库下的「查询」分组里;
导入:Navicat 的连接(本机或 .ncx 导出文件)和 Navicat 里保存的查询。

表结构设计(M5):对象树里表节点右键「设计表」、库(PostgreSQL 是 schema)右键「新建表」,
在字段 / 索引 / 外键 / 选项页里改,「SQL 预览」页实时看将要执行的 DDL,⌘S 确认后执行。
PostgreSQL、SQLite 在一个事务里执行;SQLite 做不到的改动自动走重建表;MySQL 逐条执行,中途失败会说明哪些已经生效。

导入导出(M6):表节点右键「导出数据…」「导入数据…」,表数据页和查询结果上也有「导出」按钮。
导出成 CSV(分隔符、引号、列名行、UTF-8 / 带 BOM / GBK)、Excel、JSON(数组或 JSON Lines)、SQL INSERT,
整张表、当前筛选条件下的结果、查询的全部结果都行,边读边写、内存不随行数涨,能中途取消,
完成后可以在访达中显示。导入 CSV / Excel 走三步向导(选文件 → 映射列 → 选项),
按目标列类型解析,解析不了的进失败报告(可以另存成 CSV),不悄悄改成 NULL;
PostgreSQL、SQLite 整体一个事务,MySQL 每批提交并写明哪些批已经生效。

表格快捷键:方向键移动、回车或双击编辑、Tab 提交并右移、Esc 取消、⌘C 复制、⌘⌫ 删除 / 恢复行、⌘S 提交、
右键「设为 NULL」「撤销这一格」。没有主键的表(含视图)只读。

SQL 编辑器快捷键:⌘↩ 执行光标所在的语句(有选中文字时执行选中的部分)、⇧⌘↩ 全部执行、⌘. 取消、
⌘S 保存、⇧⌘S 另存为、Ctrl-Space 手动唤出补全。每个查询标签页有自己的一条连接:
`SET @x`、临时表、`BEGIN` 开的事务都留在这个标签页里;MySQL 的存储过程不写 DELIMITER 也能整体执行。

## 运行

```bash
pnpm install
pnpm tauri dev          # 桌面应用
pnpm dev                # 只起前端;浏览器里自动使用 mock 后端,方便调界面
```

连接配置存放在 `~/Library/Application Support/dev.basalt.app/connections.json`(不含密码),
密码存在钥匙串的 `dev.basalt.app` 服务下。同一目录下还有执行历史 `history.jsonl`(最近 2000 条)
和保存的查询 `queries/<连接 id>/<库>/[<schema>/]<名字>.sql`(删除连接不会删它的查询)。

从 Navicat 迁移保存的查询:工具栏「导入」→「Navicat 保存的查询」。只读 Navicat 的
`~/Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/Settings` 下的 .sql 文件,不改它;
每条先按连接名找 Basalt 里的连接,对不上再按 Navicat 里同名连接的地址找,
预览里写明每条放到哪个连接、哪个库下,对不上的说明原因、不导入;同名的已有查询不会被覆盖。

## 测试

第一次跑之前装两样工具(都只装在用户目录):

```bash
pnpm exec playwright install webkit                                # 截图测试用的 WebKit
rustup component add llvm-tools-preview && cargo install cargo-llvm-cov --locked   # Rust 覆盖率
```

前端:

```bash
pnpm lint                                   # ESLint(typescript-eslint + react-hooks)+ Prettier 检查,零告警才算过
pnpm format                                 # 用 Prettier 格式化
pnpm typecheck
pnpm test                                   # 单测与交互测试(Vitest + Testing Library,mock 后端)
pnpm test --coverage                        # 同上并统计覆盖率,任一项低于 80% 即失败;HTML 报告在 coverage/
pnpm test:visual                            # 截图回归(Playwright + WebKit,自动在 1430 端口起 pnpm dev)
pnpm test:visual:update                     # 界面有意改动后重新生成基准图
```

截图测试覆盖欢迎页、展开的对象树、对象列表、新建连接对话框(选类型 / MySQL 表单)、
SQL 编辑器的结果表格与消息页、导入 Navicat 查询的预览、表结构设计器、导出对话框(选项 / 完成)、
导入向导(选文件 / 映射列 / 结果),
每个画面按 960、1280、1440 三种宽度和深浅两套主题各截一张,基准图在 `e2e/__screenshots__/`。
只用 WebKit,因为 Tauri 在 macOS 上用的就是它,Chromium 的排版和它有出入。
基准图依赖本机字体,换机器或升级系统后先 `pnpm test:visual:update`,再逐张看差异。
失败时的对比图在 `test-results/`,`pnpm exec playwright show-report` 可以并排查看。

Rust:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                      # Rust 单测 + SQLite 集成测试
docker compose -f docker-compose.test.yml up -d                   # MySQL 5.7 + PostgreSQL 17
docker compose -f docker-compose.test.yml --profile full up -d    # 再加 MySQL 8.4 + MariaDB 11.4
BASALT_IT=1 cargo test --workspace          # 连 MySQL 5.7 与 PostgreSQL
BASALT_IT=all cargo test --workspace        # 连全部四个容器(会在测试库里建 basalt_it / basalt_m2 / basalt_m3 / basalt_m6 / basalt_svc)
BASALT_IT=all cargo llvm-cov --workspace --fail-under-lines 80    # 覆盖率;不连容器时 MySQL / PG 驱动跑不到
```

设了 `BASALT_IT` 时还会跑 `transfer_large`:每种库 10 万行(约 100 MB)导出 CSV / Excel 并量进程内存、
中途取消、导回再取消,调试构建下要几分钟,`--nocapture` 能看到每一步的用时和内存。

测试容器只监听 127.0.0.1:MySQL 5.7 `23306`、MySQL 8.4 `23307`、MariaDB `23308`、PostgreSQL `25432`,
账号见 `docker-compose.test.yml`(仅用于本机一次性容器)。

ESLint 通过 `tools/typescript-eslint-ts6` 给 typescript-eslint 配了一份 TypeScript 6:
项目用的 TypeScript 7 是原生版,没有它需要的 JS 编译器接口。`tsc` 仍然用 7。

## 目录

```
crates/basalt-core/   连接配置、密码存储、驱动、会话、表数据读写、SQL 生成、SQL 编辑器的执行 / 历史 / 保存的查询、
                      导入导出(transfer/)(不依赖 Tauri)
src-tauri/            Tauri 外壳:commands.rs 只做转发,逻辑在 service.rs
src/                  前端:components 按界面区域组织,lib/api.ts 是后端接口
e2e/                  Playwright 截图测试与基准图
tools/                开发工具用的本地包(给 typescript-eslint 配 TypeScript 6)
```
