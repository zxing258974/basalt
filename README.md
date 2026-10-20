# Basalt

对标 Navicat Premium 使用体验的桌面数据库客户端,自用。Tauri 2 + Rust + React。
设计与里程碑见 [DESIGN.md](DESIGN.md)。

**当前进度:M2**——连接管理(新建 / 编辑 / 复制 / 删除 / 分组 / 测试连接,密码进 macOS 钥匙串)、
MySQL / MariaDB / PostgreSQL / SQLite 四种驱动、对象树(连接 → 库 → schema → 表 / 视图)、对象列表页;
数据浏览与编辑:在对象树里双击表、或在对象列表里双击一行打开「表数据」标签页,
分页、点列头排序、WHERE 筛选、行内编辑、新增 / 删除行,未提交的修改高亮,⌘S 先看 SQL 预览再在一个事务里提交。

表格快捷键:方向键移动、回车或双击编辑、Tab 提交并右移、Esc 取消、⌘C 复制、⌘⌫ 删除 / 恢复行、⌘S 提交、
右键「设为 NULL」「撤销这一格」。没有主键的表(含视图)只读。

## 运行

```bash
pnpm install
pnpm tauri dev          # 桌面应用
pnpm dev                # 只起前端;浏览器里自动使用 mock 后端,方便调界面
```

连接配置存放在 `~/Library/Application Support/dev.basalt.app/connections.json`(不含密码),
密码存在钥匙串的 `dev.basalt.app` 服务下。

## 测试

```bash
pnpm test                                   # 前端单测(Vitest)
pnpm typecheck
cargo test --workspace                      # Rust 单测 + SQLite 集成测试
docker compose -f docker-compose.test.yml up -d                   # MySQL 5.7 + PostgreSQL 17
docker compose -f docker-compose.test.yml --profile full up -d    # 再加 MySQL 8.4 + MariaDB 11.4
BASALT_IT=1 cargo test --workspace          # 连 MySQL 5.7 与 PostgreSQL
BASALT_IT=all cargo test --workspace        # 连全部四个容器(会在测试库里建 basalt_it / basalt_m2 / basalt_svc)
```

测试容器只监听 127.0.0.1:MySQL 5.7 `23306`、MySQL 8.4 `23307`、MariaDB `23308`、PostgreSQL `25432`,
账号见 `docker-compose.test.yml`(仅用于本机一次性容器)。

## 目录

```
crates/basalt-core/   连接配置、密码存储、驱动、会话、表数据读写、SQL 生成(不依赖 Tauri)
src-tauri/            Tauri 外壳:commands.rs 只做转发,逻辑在 service.rs
src/                  前端:components 按界面区域组织,lib/api.ts 是后端接口
```
