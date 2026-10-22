# Basalt

对标 Navicat Premium 使用体验的桌面数据库客户端,自用。Tauri 2 + Rust + React。
设计与里程碑见 [DESIGN.md](DESIGN.md)。

**当前进度:M0–M6**——连接管理(新建 / 编辑 / 复制 / 删除 / 分组 / 测试连接,密码存在本机文件)、
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

SSH 隧道与 SSL(M4):连接对话框分「常规 / SSH / SSL」三页。SSH 隧道支持密码、私钥(可带口令)和 SSH Agent 登录,
可以先经过一台或几台跳板机(写法同 OpenSSH 的 ProxyJump,能写 `~/.ssh/config` 里的别名),
也能在 SSH 页「从 ~/.ssh/config 填入」选一台主机自动填好地址、用户名、私钥和跳板机;
数据库的主机、端口填从 SSH 服务器那边能访问的地址;第一次连接某台 SSH 服务器时弹框核对它的主机密钥指纹,
信任后记在配置目录的 `known_hosts` 里,以后指纹变了会拒绝连接并说明怎么处理。SSL 有禁用 / 优先 / 必须 /
校验 CA / 校验主机名五档,可选 CA 证书和客户端证书、私钥。SSH 密码、私钥口令和数据库密码一样存在 `secrets.json` 里。
从 Navicat 导入的 .ncx 连接会带上隧道和 SSL 设置,映射不了的在预览里提醒。

导入导出(M6):表节点右键「导出数据…」「导入数据…」,表数据页和查询结果上也有「导出」按钮。
导出成 CSV(分隔符、引号、列名行、UTF-8 / 带 BOM / GBK)、Excel、JSON(数组或 JSON Lines)、SQL INSERT,
整张表、当前筛选条件下的结果、查询的全部结果都行,边读边写、内存不随行数涨,能中途取消,
完成后可以在访达中显示。导入 CSV / Excel 走三步向导(选文件 → 映射列 → 选项),
按目标列类型解析,解析不了的进失败报告(可以另存成 CSV),不悄悄改成 NULL;
PostgreSQL、SQLite 整体一个事务,MySQL 每批提交并写明哪些批已经生效。

表操作:表节点右键还有「转储 SQL 文件…」(建表语句加上全部数据或只要结构,能直接执行还原)、
「复制表…」「重命名…」「复制表名」「维护…」(MySQL 的分析 / 检查 / 优化 / 修复,PostgreSQL 的
VACUUM / ANALYZE / REINDEX,SQLite 的 ANALYZE / VACUUM,显示数据库返回的消息),以及「清空表…」
「截断表…」「删除表…」;清空和截断在同一个对话框里讲清区别。

表格快捷键:方向键移动、回车或双击编辑、Tab 提交并右移、Esc 取消、⌘C 复制、⌘⌫ 删除 / 恢复行、⌘S 提交、
右键「设为 NULL」「撤销这一格」。没有主键的表(含视图)只读。

SQL 编辑器快捷键:⌘↩ 执行光标所在的语句(有选中文字时执行选中的部分)、⇧⌘↩ 全部执行、⌘. 取消、
⌘S 保存、⇧⌘S 另存为、Ctrl-Space 手动唤出补全。每个查询标签页有自己的一条连接:
`SET @x`、临时表、`BEGIN` 开的事务都留在这个标签页里;MySQL 的存储过程不写 DELIMITER 也能整体执行。

查询结果:每个结果集都完整读回来放在后端,表格先显示前 1000 行,往下滚自动接着取,点列头按整个结果排序。
结果来自一张带主键的表(单表 SELECT,结果里有主键列)时可以直接改:和表数据页一样增删改行、⌘S 预览并提交,
提交后重新执行这条语句刷新结果。JOIN 了几张表时一次改一张(底栏选改哪张,这张表的主键列要在结果里),
只改格子、不增删行;子查询、UNION 的结果和表达式列只读,底栏写明原因。同名的列(两张表的 id)表头上带表名。
导出查询结果时按表格上点的排序(结果太大、导出时要重新执行的,按语句本来的顺序)。

服务器监控:MySQL / MariaDB / PostgreSQL 的连接右键「服务器监控」,打开后每 2 / 5 / 10 秒取一次(可暂停,
切到别的标签页时不取)。上面一排指标:每秒查询数(PostgreSQL 是每秒事务提交数)、回滚、连接数占上限的比例、
缓存命中率(InnoDB 缓冲池 / 共享缓冲区)、MySQL 的收发流量和慢查询数,带最近 60 次刷新的走势;
下面是会话列表,在执行的、开着事务没提交的排前面,空闲连接和服务器自己的线程(事件调度器、复制)排最后,
能隐藏空闲、按用户 / 地址 / 语句筛选,每行可以「终止查询」(只停这条语句)或「断开连接」,先确认;
确认之后会话已经换了语句或者结束了就不动手。取数用的是每个连接单独的一条连接,标着「Basalt」、不能终止。
MySQL 账号没有 PROCESS 权限时只看得到自己的会话;PostgreSQL 账号不是超级用户、没有 pg_read_all_stats 时
看得到别人的会话、看不到他们在做什么。两种情况页面上都写明。

## 运行

```bash
pnpm install
pnpm tauri dev          # 桌面应用
pnpm dev                # 只起前端;浏览器里自动使用 mock 后端,方便调界面
```

连接配置存放在 `~/Library/Application Support/dev.basalt.app/connections.json`(不含密码),
密码(含 SSH 密码、私钥口令、SQL 助手的 API Key)存在同一目录的 `secrets.json`,只有当前用户能读写(权限 600),
**是明文**:备份、网盘同步、本账号下运行的程序都能读到。信任过的 SSH 服务器主机密钥在同一目录的
`known_hosts`(OpenSSH 格式;服务器换了密钥、确认无误后删掉对应的那一行)。同一目录下还有执行历史 `history.jsonl`(最近 2000 条)
和保存的查询 `queries/<连接 id>/<库>/[<schema>/]<名字>.sql`(删除连接不会删它的查询)。
以前的版本把密码存在钥匙串的 `dev.basalt.app` 服务下:启动时按配置里存过的密码一次全部搬进 `secrets.json`,
钥匙串的授权框都在这时弹完(合并过的那一条问一次,更早单独存的每条各问一次,都点「始终允许」),搬完就再也不碰钥匙串;
中途拒绝或取消了授权框的话,已经搬过来的留着,剩下的不再搬、以后也不再问(到时重新填密码或 API Key)。钥匙串里的旧条目不会自动删,不需要了可以在「钥匙串访问」里搜
`dev.basalt.app` 手动删。

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

截图测试覆盖欢迎页、展开的对象树、对象列表、新建连接对话框(选类型 / MySQL 表单 / SSH 页 / SSL 页)、
核对 SSH 主机密钥、服务器监控、
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
docker compose -f docker-compose.test.yml up -d --build           # MySQL 5.7 + PostgreSQL 17 + sshd(第一次会构建 sshd 镜像)
docker compose -f docker-compose.test.yml --profile full up -d    # 再加 MySQL 8.4 + MariaDB 11.4
BASALT_IT=1 cargo test --workspace          # 连 MySQL 5.7、PostgreSQL 与 sshd
BASALT_IT=all cargo test --workspace        # 连全部容器(会在测试库里建 basalt_it / basalt_m2 / basalt_m3 / basalt_m6 / basalt_svc,以及证书登录的 basalt_cert 账号)
BASALT_IT=all cargo llvm-cov --workspace --fail-under-lines 80    # 覆盖率;不连容器时 MySQL / PG 驱动跑不到
```

设了 `BASALT_IT` 时还会跑 `transfer_large`:每种库 10 万行(约 100 MB)导出 CSV / Excel 并量进程内存、
中途取消、导回再取消,调试构建下要几分钟,`--nocapture` 能看到每一步的用时和内存。

测试容器只监听 127.0.0.1:MySQL 5.7 `23306`、MySQL 8.4 `23307`、MariaDB `23308`、PostgreSQL `25432`、
sshd `20022`,账号见 `docker-compose.test.yml`(仅用于本机一次性容器)。

SSH 隧道与 SSL 的测试(`tests/ssh_tunnel.rs`、`tests/ssl.rs`):sshd 是 SSH 跳板机(alpine + OpenSSH,
用户 basalt 可以密码或 `tests/fixtures/ssh` 里的测试私钥登录并转发,noforward 不许转发),和数据库容器在同一个网络,
经它连库时地址写容器名 `mysql57`、`postgres`。MySQL 5.7 和 PostgreSQL 挂着 `tests/fixtures/tls` 里测试 CA 签的证书
开 SSL,PostgreSQL 另用那里的 `pg_hba.conf` 让 `basalt_cert` 只能用客户端证书登录。M4 以前起的这两个容器要重建一次:
`docker compose -f docker-compose.test.yml up -d --force-recreate mysql57 postgres`(数据卷保留)。
`BASALT_IT` 的名单里写 `sshd` 选中跳板机,经隧道的测试要同时选中 sshd 和对应的库。
隧道本身的单元测试不需要容器:在测试进程里用 russh 的服务端起 SSH 服务器。

Windows:

```bash
tools/check-windows.sh                      # 在 Mac 上对 Windows 目标跑 clippy(C 依赖用假编译器顶替,不链接)
```

Windows 安装包(NSIS 的 .exe 和 MSI)由 `.github/workflows/windows.yml` 在 GitHub 的 Windows 机器上打,
手动触发或推 `v` 开头的 tag;打完包再在 Windows 上跑 `cargo test --workspace`(不连数据库容器)。
平台差异:密码存 Windows 凭据管理器、快捷键用 Ctrl、「在资源管理器中显示」、没有红绿灯的标题栏、
WebView2 的刷新 / 打印 / 页内查找快捷键和空白处右键菜单都关掉;本机 Navicat 来源在 Windows 上不可用,只能导 .ncx。
前端测试固定按 macOS 跑(`src/test/setup.ts`),Windows 的文案和按键另有测试。

ESLint 通过 `tools/typescript-eslint-ts6` 给 typescript-eslint 配了一份 TypeScript 6:
项目用的 TypeScript 7 是原生版,没有它需要的 JS 编译器接口。`tsc` 仍然用 7。

## 目录

```
crates/basalt-core/   连接配置、密码存储、驱动、会话、表数据读写、SQL 生成、SQL 编辑器的执行 / 历史 / 保存的查询、
                      导入导出(transfer/)(不依赖 Tauri)
src-tauri/            Tauri 外壳:commands.rs 只做转发,逻辑在 service.rs
src/                  前端:components 按界面区域组织,lib/api.ts 是后端接口
e2e/                  Playwright 截图测试与基准图
tools/                开发工具:给 typescript-eslint 配 TypeScript 6 的本地包、Windows 编译检查脚本
```
