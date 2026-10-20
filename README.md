# Basalt

对标 Navicat Premium 使用体验的桌面数据库客户端,自用。Tauri 2 + Rust + React。
设计与里程碑见 [DESIGN.md](DESIGN.md)。

**当前进度:M1**——连接管理(新建 / 编辑 / 复制 / 删除 / 分组 / 测试连接,密码进 macOS 钥匙串)、
MySQL / MariaDB / PostgreSQL / SQLite 四种驱动、对象树(连接 → 库 → schema → 表 / 视图)、对象列表页。

## 运行

```bash
pnpm install
pnpm tauri dev          # 桌面应用
pnpm dev                # 只起前端;浏览器里自动使用 mock 后端,方便调界面
```

连接配置存放在 `~/Library/Application Support/dev.basalt.app/connections.json`(不含密码),
密码存在钥匙串的 `dev.basalt.app` 服务下。

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

截图测试覆盖欢迎页、展开的对象树、对象列表、新建连接对话框(选类型 / MySQL 表单),
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
BASALT_IT=all cargo test --workspace        # 连全部四个容器
BASALT_IT=all cargo llvm-cov --workspace --fail-under-lines 80    # 覆盖率;不连容器时 MySQL / PG 驱动跑不到
```

测试容器只监听 127.0.0.1:MySQL 5.7 `23306`、MySQL 8.4 `23307`、MariaDB `23308`、PostgreSQL `25432`,
账号见 `docker-compose.test.yml`(仅用于本机一次性容器)。

ESLint 通过 `tools/typescript-eslint-ts6` 给 typescript-eslint 配了一份 TypeScript 6:
项目用的 TypeScript 7 是原生版,没有它需要的 JS 编译器接口。`tsc` 仍然用 7。

## 目录

```
crates/basalt-core/   连接配置、密码存储、驱动、会话、SQL 工具(不依赖 Tauri)
src-tauri/            Tauri 外壳:commands.rs 只做转发,逻辑在 service.rs
src/                  前端:components 按界面区域组织,lib/api.ts 是后端接口
e2e/                  Playwright 截图测试与基准图
tools/                开发工具用的本地包(给 typescript-eslint 配 TypeScript 6)
```
