# Basalt 设计文档

> 一个对标 Navicat Premium 使用体验的桌面数据库客户端,给自己用。
> v0.3 · 2026-10-20 · M0、M1、M2 已完成

## 1. 目标与边界

**目标**:日常开发里替代 Navicat 最常用的那部分——连上库、看表、改数据、写 SQL、改表结构、导入导出。

**第一期支持**:MySQL 5.7+/8.x、MariaDB 10.x/11.x、PostgreSQL 12+、SQLite 3。

**第一期功能**:

| 模块 | 内容 |
|---|---|
| 连接管理 | 新建/编辑/复制/删除连接,分组,测试连接,密码进 macOS 钥匙串,SSH 隧道(密码/私钥) |
| 对象树 | 连接 → 数据库 →(PG 的 schema)→ 表 / 视图;对象列表页显示行数、大小、引擎、注释 |
| 数据浏览与编辑 | 分页表格、排序、筛选(WHERE)、行内编辑、新增/删除行,按主键生成 UPDATE/DELETE,提交前可看 SQL |
| SQL 编辑器 | 方言高亮、按库结构补全、执行全部 / 选中 / 当前语句、多结果集标签、消息与耗时、取消执行、历史 |
| 表结构设计 | 字段 / 索引 / 外键编辑,按方言生成 CREATE / ALTER,执行前 SQL 预览 |
| 导入导出 | 表或查询结果导出 CSV / XLSX / JSON / SQL INSERT;CSV / XLSX 导入(列映射);执行 SQL 文件 |

**明确不做**:

- 不反编译、不照搬 Navicat 的代码与资源,功能设计依据公开文档与使用观察,代码全部自写。
- 不用 Navicat 的名称、图标、配色作为产品标识。
- 不碰任何授权 / 激活相关的东西。
- 第一期不做:Oracle / SQL Server / MongoDB / Redis、ER 图建模、数据 / 结构同步、备份还原、计划任务、云同步。

## 2. 技术选型

| 层 | 选择 | 理由 |
|---|---|---|
| 桌面壳 | Tauri 2 | 安装包小、内存占用低,Rust 后端直接持有数据库连接 |
| 数据库驱动 | sqlx 0.8(mysql / postgres / sqlite,tokio + rustls) | 一套 API 覆盖三种库;MariaDB 走 mysql 协议;不用编译期宏,不依赖 DATABASE_URL。0.9 要求 Rust 1.94(本机 1.93),暂不升 |
| SSH 隧道 | russh | 纯 Rust、异步;本地起 127.0.0.1 随机端口做 direct-tcpip 转发,驱动层无感 |
| 密码存储 | keyring(apple-native) | 连接配置文件里不落明文密码 |
| 导入导出 | csv、rust_xlsxwriter、calamine | 都是成熟库 |
| 前端 | React 19 + TypeScript + Vite | |
| 服务端状态 | TanStack Query | 元数据、查询结果按 key 缓存与失效 |
| 客户端状态 | Zustand | 标签页、选中节点、界面偏好 |
| 表格 | TanStack Table + TanStack Virtual | 无头组件,虚拟滚动扛得住几万行,编辑交互自己掌控 |
| SQL 编辑器 | CodeMirror 6 + @codemirror/lang-sql | 自带 MySQL / PG / SQLite 方言与基于 schema 的补全,比 Monaco 轻 |

## 3. 架构

```
basalt/
├── crates/basalt-core/      纯 Rust 核心:不依赖 Tauri,可单测、可集成测
│   ├── config/              连接配置模型 + JSON 仓库(不可变更新)
│   ├── secret.rs            SecretStore trait;钥匙串实现 + 内存实现(测试用)
│   ├── driver/              三种库的连接、元数据查询;*_data.rs 是表数据(列信息、按类型解码、事务执行)
│   ├── value.rs             CellValue:统一的单元格值表示
│   ├── table.rs             表数据的请求 / 响应结构(TableRef、ColumnInfo、PageRequest、RowChange…)
│   ├── browse.rs            Session 上的读一页 / 预览 / 提交,把列信息、SQL 生成与驱动串起来
│   ├── sql/                 纯函数:筛选条件检查(filter)、分页 SELECT(select)、DML 生成(dml / bind)
│   └── tunnel.rs            (M4)SSH 本地端口转发
├── src-tauri/               Tauri 外壳:命令层只做参数校验、调 core、错误转换
└── src/                     前端
    ├── components/          按界面区域组织(layout / sidebar / connection / objects / data / ...)
    ├── lib/api.ts           invoke 的类型化封装;开发模式可切到 mock 后端
    ├── stores/              Zustand
    └── styles/tokens.css    设计令牌
```

**连接生命周期**:配置(磁盘 JSON)→ 打开(建隧道 → 建连接池)→ 会话表(内存,按连接 id)→ 关闭(关池 → 关隧道)。
PostgreSQL 一个连接只能连一个库,所以会话里按「库名」懒建连接池;MySQL 一个池跨库,SQL 里显式带库名限定;SQLite 一个文件一个池。

**值的表示**:驱动层把每个单元格解码成 `CellValue`(Null / Bool / Int / UInt / Float / Decimal / Text / Bytes / Json / Date / Time / DateTime / Unsupported{type})。前端只按这个枚举渲染与编辑;写回时按列类型生成参数化 SQL。解码不了的类型显示类型名,不静默丢弃。整数、浮点数在 JSON 里一律是字符串(JS 的 number 装不下 BIGINT),字节是十六进制。

**数据浏览与编辑(M2)**:

- **读一页**:SELECT 显式列出表结构里的列,`LIMIT n+1` 判断后面还有没有(不做 COUNT(*));排序列之后追加主键打破平局,没指定排序时按主键排,翻页不重不漏。筛选框里的 WHERE 原样拼进去,但先做词法检查:跳过字符串、带引号标识符和注释后不许有分号(SQLite 会执行分号后的语句)、括号要配对、引号注释要闭合;条件单独成行包在括号里,末尾的 `--` 注释吞不掉 LIMIT。
- **按类型解码**:按 `type_info().name()` 分派。MySQL 的零日期、TIME 负值与超 24 小时、utf8mb4_bin 文本列(线上被报成 VARBINARY)都按原始字节自己解;PostgreSQL 的 NUMERIC(超过 rust_decimal 的 28 位)、DATE / TIMESTAMP 的 infinity 与公元前也自己解;SQLite 按每个值自己的存储类型解。PostgreSQL 里驱动不直接解的类型(数组、区间、inet、money、枚举、几何……)在 SELECT 里 `::text` 读出,写回时 `$1::原类型` 交给数据库解析,所以都能看也能改;这时 ORDER BY 要带表名限定,否则会按转成 text 的输出列排序。
- **会话设置**:sqlx 默认会改会话——MySQL 追加 `PIPES_AS_CONCAT`(`||` 变成字符串拼接)、`NO_ENGINE_SUBSTITUTION` 并把 time_zone 设成 `+00:00`(`DEFAULT CURRENT_TIMESTAMP` 会存成 UTC),PostgreSQL 把 TimeZone 设成 UTC。MySQL 这三项全部关掉,保持服务器默认行为;PostgreSQL 改成本机 IANA 时区,timestamptz 按本机时区带偏移显示,筛选条件里的时间字面量与显示一致。
- **写回**:一律参数化、按主键定位;没有主键(含视图)只读并说明原因,生成列、解不了的值不能编辑。PostgreSQL 所有参数以文本传入加 `::类型`(`format_type(oid, -1)`,char 列得到 bpchar 而不是 character(1),不会被截断);MySQL 的主键按原值类型绑定(DECIMAL 用定点数),避免字符串与数字比较退化成浮点比较而匹配错行,新值在客户端先校验数值格式,免得非严格模式把错误输入存成 0;SQLite 传文本,由列亲和性转换。二进制列的输入以 0x 开头时按十六进制解析。
- **提交**:同一批修改按「删除 → 更新 → 新增」排序后在一个事务里执行,每条语句必须恰好影响 1 行(MySQL 连接开了 FOUND_ROWS,值没变的 UPDATE 也算命中),否则整批回滚并说明是第几条。预览里参数内联成字面量,只用于显示;每次都重新读列信息,不信任前端手里的表结构,提交时带上用户确认过的预览,重新生成的语句不一样(预览之后表结构变了)就拒绝执行。MyISAM、MEMORY 等不支持事务的引擎一批只许一条语句,否则中途失败无法回滚。COMMIT 本身失败时如实说「无法确定是否生效」,不说已回滚。
- **筛选的第二道防线**:SQLite 会执行一个字符串里的所有语句,读页时连接临时开 `PRAGMA query_only`;MySQL、PostgreSQL 用预备语句,本来就拒绝多条语句。
- **前端**:修改按「本页行号」记录,有未提交的修改时锁住翻页、排序、筛选、刷新,⌘S 先出预览再提交,提交失败修改原样保留;提交成功后等数据重读完才关对话框,不闪回旧值。关闭有未提交修改的标签页(×、中键、⌘W)或关闭它所在的连接前先确认。文本列里出现的二进制值(无效 UTF-8、SQLite 动态类型)只读:写回时 0x 不会被当十六进制。表格用 TanStack Virtual 只渲染可见行,编辑框是 textarea(input 会悄悄吞掉换行),输入法组字时的回车不算提交。表格的键盘焦点是一个看不见、盖在选中格上的 textarea(GridInput):方向键和快捷键由表格处理,可打印的键不拦,让字照常进框——拼音等输入法因此能组字,组字时框显示成编辑框的样子、候选窗贴着选中格;文字上屏(非组字的输入,或 compositionend)后才带着它打开编辑框,keyCode 229 的键一律归输入法。读屏靠框上的 aria-activedescendant 读当前格(WebKit 映射成 AXActiveElement,选区移动时发 AXActiveElementChanged);role=grid 挪到滚动容器里面一层,滚动容器 tabIndex -1、一获得焦点就转给输入框,自己不占 Tab 位。输入框不用 readOnly(焦点不动时切换 readOnly,WKWebView 不一定重新判断要不要交给输入法):只读格照样接键盘,打出的字由表格丢掉,也不显示组字框;输入框失焦时复位组字状态,失焦后才上屏的字不开编辑框。所有标签页保持挂载、只显示活动的一个,切走再回来修改和滚动位置都在。

**已知限制(M2)**:大 BLOB / TEXT 整值传给前端,不截断;inet 转 text 总带掩码(如 10.0.0.1/32);选中格子直接打中文已支持(简体拼音在真窗口验证过组字、空格选字、回车上屏字母、Esc 取消、中文标点直接上屏),读屏的当前格只在辅助功能接口层核对过(焦点框的 AXActiveElement 指向选中格),没开 VoiceOver 实际试听;MySQL 以 FLOAT 列或超过 28 位有效数字的 DECIMAL 列为主键的行无法定位,只能看不能改。

**错误**:core 用 thiserror 定义错误;命令层统一转成 `{ code, message }` 给前端,message 写人话(连不上、认证失败、超时、SQL 错误原文)。

**安全**:前端永远拿不到密码;SQL 编辑器执行的是用户自己的 SQL,但表格编辑生成的 SQL 一律参数化;标识符按方言转义(反引号 / 双引号)。

## 4. 界面

整体沿用 Navicat 的「左树右标签」心智模型,但视觉自成一套:

- **风格方向**:macOS 原生工具感 + 瑞士风格的克制排版。浅色为主、深色可切;数据区等宽字体,界面区系统字体(刻意贴近原生,不另起字体)。
- **色彩语义化**:每种数据库一个识别色(MySQL 青、PG 蓝、SQLite 灰绿),只用在连接图标与标签页左边条;危险操作(删除、执行 DDL)用统一的警示色。
- **布局**:顶部工具栏(连接 / 新建查询 / 表 / 视图 / 导入导出)→ 左侧对象树(可拖宽)→ 右侧标签页区(对象列表、表数据、查询、设计器)→ 底部状态栏(当前连接、耗时、行数)。
- **键盘**:⌘N 新建连接、⌘T 新建查询、⌘↩ 执行、⌘S 提交修改、⌘W 关闭标签。

## 5. 测试策略

- **core 单测**:配置仓库、值解码、标识符转义、语句切分、DML / DDL 生成——纯函数先写测试。
- **core 集成测**:`docker-compose.test.yml` 起 MySQL 5.7 / 8.4、MariaDB 11.4、PostgreSQL 17 和 sshd,测试按环境变量连接,没起容器时跳过。
- **前端**:Vitest + Testing Library 测组件逻辑;开发模式的 mock 后端让界面能在浏览器里跑,用 Playwright 截图核对布局。
- **真机**:每个里程碑结束在 Tauri 窗口里手动走一遍主流程。

## 6. 里程碑

| | 内容 | 完成标准 |
|---|---|---|
| M0 ✅ | 工程骨架:Tauri + React + core crate、设计令牌、主布局、测试与容器环境 | `pnpm tauri dev` 起得来,三层测试都能跑 |
| M1 ✅ | 连接管理 + 三种驱动 + 对象树 + 对象列表 | 能新建并保存 MySQL / PG / SQLite 连接,展开到表,看到表信息 |
| M2 ✅ | 数据浏览与编辑 | 分页、排序、筛选,改一行提交成功,SQL 预览正确 |
| M3 | SQL 编辑器 | 补全可用,多语句多结果集,能取消长查询 |
| M4 | SSH 隧道 + SSL | 经 sshd 容器连上 MySQL / PG |
| M5 | 表结构设计 | 三种方言的建表 / 改表 SQL 生成通过单测与真库验证 |
| M6 | 导入导出 | 四种格式导出、两种格式导入往返一致 |
| M7 | 打磨与打包 | 深色模式、快捷键、错误提示、打出 .app |
