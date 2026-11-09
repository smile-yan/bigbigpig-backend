# Smileyan 博客后端（Rust 实现）

[![CI](https://github.com/smile-yan/smileyan-backend-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/smile-yan/smileyan-backend-rs/actions/workflows/ci.yml)
[![Release](https://github.com/smile-yan/smileyan-backend-rs/actions/workflows/release.yml/badge.svg)](https://github.com/smile-yan/smileyan-backend-rs/actions/workflows/release.yml)

这是 [smileyan-backend](../smileyan-backend) 的 Rust 重写实现。目标是与 Go/Gin
基线**行为等价**，而不是借重写之机调整 API：

- 路径、HTTP 方法、认证层级、状态码、错误文案、响应 JSON 形状保持不变；
- 数据库 schema、软删除语义、slug 规则、Redis key/TTL 保持不变；
- JWT 与 Go 版双向互通，现有 token 继续有效；
- Markdown 渲染结果与 Go 基线对齐（25 个 golden 用例，仅两类已记录的语义等价差异）；
- 前端（Vue 3）无需任何修改。

> 基线：`smileyan-backend` tag `go-baseline-2026-11`（commit `1f3a87f`）。
> 当前实现已完成 Phase 0–8，共 43 个 API 端点（另有 `/uploads/*` 静态文件路由）；
> `tests/contract_full.rs` 会按采集顺序重放全部 172 个 HTTP、8 封邮件和 13 个
> note 夹具。

## 目录

- [功能概览](#功能概览)
- [实现状态](#实现状态)
- [技术栈](#技术栈)
- [快速开始](#快速开始)
- [配置说明](#配置说明)
- [构建与打包](#构建与打包)
- [测试与质量检查](#测试与质量检查)
- [API 概览](#api-概览)
- [兼容性说明](#兼容性说明)
- [性能](#性能)
- [部署与回滚](#部署与回滚)
- [项目结构](#项目结构)
- [文档索引](#文档索引)
- [契约夹具（Phase 0）](#契约夹具phase-0)
- [已知限制与后续项](#已知限制与后续项)

## 功能概览

- **认证与用户**：邮箱验证码登录（Redis 600 秒 TTL + 邮箱/IP 限流）、
  HS256 JWT、当前用户信息、头像上传、管理员用户列表与角色修改、`genjwt`
  开发工具。
- **内容管理**：文章、分类、标签、页面的公共与管理员 CRUD；分页、分类/标签/
  关键词/日期筛选；中文 slug 生成；文章与页面的软删除/恢复；浏览计数。
- **Markdown**：基于 comrak（GFM、Linkify、Typographer），复刻 Go 版
  `postProcessHTML`；`rebuild_html` 可在修改渲染管线后重建 `html_content`。
- **评论与订阅**：公共评论列表（一层 `children`）、评论默认待审核、管理员
  审核/删除、回复邮件通知、订阅/退订（32 位十六进制 token）、新文章并发通知。
- **搜索**：tantivy 内存索引，启动时重建；jieba 分词 + CJK 单字字段；
  `/api/search` 返回与 Go 一致的命中集合和 `total`；可选用 MySQL `LIKE` 兜底。
- **TTS**：OpenAI 兼容的 `/v1/audio/speech` 代理，支持二进制与 SSE 流式响应、
  上游错误透传、连接/总超时控制。
- **静态文件**：`/uploads` 直接提供头像、封面等上传文件。
- **运维能力**：musl 静态制品、staging（8081）、原子部署与回滚脚本、
  冒烟测试、回滚演练。

## 实现状态

| 阶段 | 内容 | 状态 |
|---|---|---|
| Phase 0 | 冻结 Go 基线；采集 172 HTTP + 8 邮件 + 13 note 夹具、25 个 Markdown golden、54 条搜索语料；输出 Q1–Q27 兼容性决策表 | 完成 |
| Phase 1 | Cargo 工程、配置/日志、MySQL/Redis、错误模型、路由骨架、migrations、testcontainers | 完成 |
| Phase 2 | 认证与用户：验证码、JWT、限流、SMTP、用户信息、头像、管理员用户管理 | 完成 |
| Phase 3 | 内容域：文章/分类/标签/页面、slug、软删除、Markdown、`rebuild_html` | 完成 |
| Phase 4 | 评论与订阅：审核、回复通知、退订 token、订阅者管理、并发通知 | 完成 |
| Phase 5 | 搜索：tantivy 索引、jieba/CJK、生命周期同步、`LIKE` 兜底 | 完成 |
| Phase 6 | TTS 代理、上传模块拆分、`/uploads` 静态文件 | 完成 |
| Phase 7 | 全量契约重放、Go/Rust 压测、`cargo audit`/`cargo deny`、前端 E2E、文档 | 完成 |
| Phase 8 | musl release、staging、部署/回滚脚本、冒烟与演练 | 完成 |

## 技术栈

| 领域 | 选型 |
|---|---|
| 语言/运行时 | Rust 1.96（edition 2024，`rust-toolchain.toml` 固定）、Tokio |
| Web 框架 | axum 0.8 + tower / tower-http |
| 数据库 | MySQL 8.4，sqlx 0.9（`mysql-rsa` 支持 `caching_sha2_password`） |
| 缓存 | Redis 7.4，`redis` 1.8 的 `ConnectionManager` |
| 认证 | `jsonwebtoken` 11（仅 HS256，`rust_crypto`） |
| 邮件 | `lettre` 0.11（SMTP，rustls + ring） |
| Markdown | comrak 0.59（关闭默认 feature，避免引入 CLI/syntect/onig） |
| 搜索 | tantivy 0.27 + tantivy-jieba 0.21 |
| HTTP 客户端 | reqwest 0.13（`rustls-no-provider` + 进程级 ring provider） |
| 配置/日志 | config 0.15 + dotenvy；tracing + 自定义按日 JSON 文件日志 |
| 测试 | testcontainers 0.27 + testcontainers-modules 0.15、wiremock、insta |

所有 TLS 路径使用 rustls + ring，musl 静态构建不会引入 `openssl-sys`、
`aws-lc-sys` 或 `onig_sys`。

## 快速开始

### 环境要求

- Rust 1.96（`rust-toolchain.toml` 会自动选择）；
- Docker（用于一次性 MySQL/Redis/MailHog，或运行集成测试）；
- 生产/长期运行需要 MySQL 8.4、Redis 7.4 和可用的 SMTP 服务。

### 1. 启动本地依赖

仓库自带 Phase 0 的一次性基础设施。下面把宿主端口对齐 `config.yaml` 的默认值；
如果本机已有 MySQL/Redis，请改用 compose 的默认高位端口并同步修改
`config.yaml`：

```bash
# 把宿主端口对齐 config.yaml 的默认值：MySQL 3306、Redis 6379、SMTP 1025
CAPTURE_MYSQL_PORT=3306 CAPTURE_REDIS_PORT=6379 CAPTURE_SMTP_PORT=1025 \
  docker compose -f tests/fixtures/docker-compose.yml up -d mysql redis mailhog
```

该 compose 会创建数据库 `smileyan_capture`，MySQL root 密码和 Redis 密码都是
`capturepass`。MailHog 的 Web UI 在 <http://127.0.0.1:18025>。

### 2. 配置环境变量

```bash
cp .env.example .env
# 编辑 .env，至少填入数据库密码、Redis 密码和 JWT secret
```

或直接在 shell 中导出：

```bash
export SMILEYAN_BACKEND_DB_HOST=127.0.0.1
export SMILEYAN_BACKEND_DB_PASSWORD=capturepass
export SMILEYAN_BACKEND_DB_NAME=smileyan_capture
export SMILEYAN_BACKEND_REDIS_HOST=127.0.0.1
export SMILEYAN_BACKEND_REDIS_PASSWORD=capturepass
export SMILEYAN_BACKEND_JWT_SECRET=dev-secret
```

`.env` 会被自动加载（`.env` 已在 `.gitignore` 中）。只有非空的环境变量才会覆盖
`config.yaml` 中的对应字段。

### 3. 启动服务

```bash
cargo run
curl -s http://127.0.0.1:8080/api/hello
# {"message":"Hello, World!"}
```

启动时会自动执行 migrations、创建默认管理员、创建上传目录并重建搜索索引。
`RUN_MIGRATIONS=false` 可以关闭 migrations。

### 4. 使用 MailHog 接收验证码邮件

默认 `config.yaml` 指向腾讯企业邮箱（465/SSL）。本地使用 MailHog 时，把
`email.host` 改为 `127.0.0.1`、`email.port` 改为 `1025`、`email.use_ssl`
改为 `false`，即可在 MailHog UI 中看到验证码邮件。未配置可用邮件服务时，
`POST /api/send-code` 会返回 Go 版的 `Failed to send email`。

### 5. 本地进程管理

```bash
./bin/start      # 后台启动，日志 logs/app.log，PID 写入 logs/app.pid
./bin/stop       # 优先按 PID 停止，其次只停止配置端口上的监听进程
./bin/restart
./bin/dev        # 有 cargo-watch 时热重载，否则单次 cargo run
```

这些脚本都按 `config.yaml` 中的端口工作；`bin/start` 在开发环境会先释放被占用
的端口。它们不会使用 `pkill -f`，因此 staging（8081）与生产（8080）可以同机
运行。

### 6. 常用工具

```bash
# 生成与 Go 版 cmd/genjwt 相同的 admin/dev token
SMILEYAN_BACKEND_JWT_SECRET=dev-secret cargo run --bin genjwt

# 修改 Markdown 渲染管线后，重建文章与页面的 html_content
cargo run --bin rebuild_html
```

## 配置说明

### config.yaml

| 配置段 | 字段 | 说明 |
|---|---|---|
| `site_url` | — | 邮件中绝对链接的前缀，默认 `https://smileyan.cn` |
| `server` | `port`、`mode`、`timezone` | 监听端口；`debug`/`prod`；时间序列化时区，默认 `Asia/Shanghai` |
| `database` | `host`、`port`、`user`、`password`、`dbname`、`charset`、`max_idle_conns`、`max_open_conns` | MySQL 连接与连接池 |
| `redis` | `host`、`port`、`username`、`password`、`db` | Redis 连接 |
| `email` | `host`、`port`、`username`、`password`、`use_ssl` | SMTP；`use_ssl: true` 为隐式 TLS（465），`false` 为 STARTTLS/明文（MailHog） |
| `jwt` | `secret`、`expire_hours` | JWT 密钥与有效期，默认 168 小时（7 天） |
| `admin_emails` | — | 管理员邮箱列表；管理员邮箱跳过邮箱限流 |
| `upload` | `path`、`avatar_path`、`max_size` | 上传根目录、头像目录、单文件大小上限（MB） |
| `tts` | `base_url`、`api_key`、`default_model` | OpenAI 兼容 TTS 上游；留空时 `/v1/audio/speech` 返回 500 |
| `search` | `fallback_like` | 索引零命中时是否回退到 MySQL `LIKE`，默认 `true` |

### 环境变量

| 变量 | 对应配置 | 必填 | 说明 |
|---|---|---|---|
| `SMILEYAN_BACKEND_DB_HOST` | `database.host` | 是 | |
| `SMILEYAN_BACKEND_DB_USER` | `database.user` | 是 | |
| `SMILEYAN_BACKEND_DB_PASSWORD` | `database.password` | 是 | 不会写入日志 |
| `SMILEYAN_BACKEND_DB_NAME` | `database.dbname` | 是 | |
| `SMILEYAN_BACKEND_REDIS_HOST` | `redis.host` | 是 | |
| `SMILEYAN_BACKEND_REDIS_USERNAME` | `redis.username` | 线上必填 | 本地可空 |
| `SMILEYAN_BACKEND_REDIS_PASSWORD` | `redis.password` | 是 | 不会写入日志 |
| `SMILEYAN_BACKEND_EMAIL_PASSWORD` | `email.password` | 建议填 | 缺失时启动仅告警，发信会失败 |
| `SMILEYAN_BACKEND_JWT_SECRET` | `jwt.secret` | 是 | 与 Go 版共用，旧 token 继续有效 |
| `SMILEYAN_BACKEND_DEV_TOKEN` | 运行时字段 | 否 | 仅本地开发；生产不要设置 |
| `SMILEYAN_BACKEND_TTS_BASE_URL` | `tts.base_url` | 否 | 留空则 TTS 返回 500 |
| `SMILEYAN_BACKEND_TTS_API_KEY` | `tts.api_key` | 否 | 不会写入日志 |
| `SMILEYAN_BACKEND_TTS_DEFAULT_MODEL` | `tts.default_model` | 否 | 请求未指定 model 时使用 |

以下字段**只从 YAML 读取**，环境变量不会覆盖：`database.port`、`redis.port`、
`redis.db`、`email.host/port/username/use_ssl`、`jwt.expire_hours`、`upload.*`、
`admin_emails`、`search.fallback_like`。如果服务不在默认端口，请直接修改
`config.yaml`。

### 启动流程

1. 读取 `.env` 与 `config.yaml`，应用非空的 `SMILEYAN_BACKEND_*` 覆盖；
2. 初始化日志：控制台 + `logs/YYYY-MM-DD.log`（JSON，按本地日期滚动）；
3. 校验必填配置（数据库 host/user/dbname、Redis host、JWT secret、时区）；
4. 连接 MySQL；连接 Redis 并执行 `PING`；
5. 执行 migrations（`RUN_MIGRATIONS=false` 可关闭）；
6. `users` 表为空时插入默认管理员；
7. 创建 `uploads/`、`uploads/avatars/`；
8. 从已发布且未删除的文章重建 tantivy 内存索引；
9. 监听 `0.0.0.0:<port>`，收到 SIGTERM/SIGINT 后优雅退出。

### 配置注意事项

- 配置缺失时，Rust 的默认值以仓库中的 `config.yaml` 为准；Go 版在 YAML 缺字段
  时会得到零值。
- `server.timezone` 固定用于 `created_at`/`updated_at` 的序列化。Go 使用进程
  本地时区；生产为 `Asia/Shanghai`，显式固定可保证不同主机上的输出一致。
- 搜索索引只存在于内存中，每次启动从数据库重建。写请求在数据库提交后同步索引；
  索引失败只记录日志，不会让 HTTP 响应失败（与 Go 一致）。
- `SMILEYAN_BACKEND_DEV_TOKEN` 不是 YAML 字段。只有签名正确且 `dev` claim 与
  该值一致的 token 才会获得合成管理员身份。
- TTS 未配置（`base_url` 或 `api_key` 为空）时，`/v1/audio/speech` 返回
  `500 {"error":"TTS service not configured"}`；启动日志只记录“是否配置”，
  不会输出 key。
- `sqlx` 启用 `mysql-rsa`，以支持 MySQL 8 的 `caching_sha2_password` 明文 TCP
  登录（生产数据库不使用 TLS）。
- `lettre` 使用 `tokio1-rustls` + `ring` + `webpki-roots`；`reqwest` 使用
  `rustls-no-provider`，并在 `AppState::connect` 中安装进程级 ring provider。

## 构建与打包

```bash
# 本机 release 构建
cargo build --locked --release

# 静态 musl 构建（推荐用 cross）
cargo install cross --git https://github.com/cross-rs/cross
cross build --locked --release --target x86_64-unknown-linux-musl

# 或使用 cargo-zigbuild（本机已安装 zig 时）
cargo zigbuild --locked --release --target x86_64-unknown-linux-musl

# 本地打包：依次尝试 cross → zigbuild → 本机 host 构建
./bin/build-and-package.sh all       # all | dev | prod | staging
```

打包输出在 `output/`，每个包恰好包含三个文件：

```
smileyan-backend      # 静态二进制
config.yaml           # prod/dev 配置；staging 为 8081
deploy-remote.sh      # 服务端部署脚本
```

`prod` 包的 `config.yaml` 为 `mode: prod`，`staging` 包为 `server.port: 8081`。
如果本机没有 cross/zigbuild，脚本会回退到 host 构建并明确提示，该二进制只适合
本地测试。CI 的 release workflow 会额外检查静态链接（`readelf -d` 无
`NEEDED`）和 80 MiB 大小上限。

## 测试与质量检查

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets                 # 需要 Docker
cargo test --test contract_full          # 193 个 Phase 0 夹具全量重放
cargo build --locked --release
cargo audit                              # 读取 .cargo/audit.toml
cargo deny check                         # 读取 deny.toml
./scripts/bench.sh                       # Go 基线 vs Rust 压测
```

集成测试使用 testcontainers 启动一次性 MySQL 8.4 与 Redis 7.4，并：

- 等待真实的 `SELECT 1`/`PING`，而不是只探测端口；
- 固定使用 `127.0.0.1` 连接，避免 `localhost` 解析到 IPv6 导致间歇失败；
- 用跨进程文件锁串行化测试二进制，避免同时启动大量容器；
- 使用固定宿主端口 `23306`/`26379`（可用 `TEST_MYSQL_PORT`/`TEST_REDIS_PORT`
  覆盖）。不要改回 Docker 随机高位端口，它会与 `TIME_WAIT` 套接字冲突；
- `testcontainers` 固定为 0.27，因为 `testcontainers-modules` 0.15 要求该版本
  （异步 Runner API 与 0.28 相同）。

`tests/contract_full.rs` 是最终的契约门禁：它从 Phase 0 种子数据开始，按采集
顺序重放 172 个 HTTP 请求、8 封邮件和 13 个 note，对时间戳/JWT/token/随机
slug 做与采集工具相同的归一化，然后逐字段比较。只有
`docs/api-differences.md` 中批准的差异会被显式调整；其他任何不同都会以夹具名
报错。Markdown golden（25 例）和搜索语料（54 条查询）也包含在测试中。

Phase 7 的审计结果：`cargo audit` 退出码为 0；`.cargo/audit.toml` 记录了两条已评估的公告
（RSA/ECDSA 路径，HS256 不会触达），唯一的 warning 是 testcontainers 的
dev-only `astral-tokio-tar` unmaintained 提示。`cargo deny check` 覆盖
advisories、licenses、bans、sources 四节。

## API 概览

共 43 个 API 端点（另有 `/uploads/*` 静态文件路由），路径、方法与认证层级与
Go 基线完全一致。

| 资源 | 公共 | 需登录 | 管理员 |
|---|---|---|---|
| 健康检查 | `GET /api/hello` | — | — |
| 认证 | `POST /api/send-code`、`POST /api/login` | — | — |
| 用户 | — | `GET/PUT /api/user`、`POST /api/upload-avatar` | `GET /api/admin/users`、`PUT /api/admin/users/{id}/role` |
| 文章 | `GET /api/posts`、`GET /api/posts/{slug}` | — | `GET/POST /api/admin/posts`、`GET /api/admin/posts/slug/{slug}`、`GET/PUT/DELETE /api/admin/posts/_id/{id}`、`POST /api/admin/posts/_id/{id}/restore` |
| 分类 | `GET /api/categories` | — | `POST /api/admin/categories`、`PUT/DELETE /api/admin/categories/_id/{id}` |
| 标签 | `GET /api/tags` | — | `POST /api/admin/tags`、`DELETE /api/admin/tags/_id/{id}` |
| 页面 | `GET /api/pages`、`GET /api/pages/{slug}`（可选认证） | — | `POST /api/admin/pages`、`GET/PUT/DELETE /api/admin/pages/_id/{id}` |
| 评论 | `GET /api/comments` | `POST /api/comments` | `GET /api/admin/comments`、`POST /api/admin/comments/_id/{id}/approve`、`POST /api/admin/comments/_id/{id}/reject`、`DELETE /api/admin/comments/_id/{id}` |
| 订阅 | `POST /api/subscribe`、`GET /api/unsubscribe` | — | `GET /api/admin/subscriptions`、`DELETE /api/admin/subscriptions/_id/{id}`、`POST /api/admin/notify` |
| 搜索 | `GET /api/search` | — | — |
| TTS | — | `POST /v1/audio/speech` | — |
| 静态文件 | `GET /uploads/*` | — | — |

### 响应约定

- 成功响应是**裸对象/数组**，没有 `{data: ...}` 包装。
- 列表响应为 `{list, total, page[, page_size]}`；分类、标签、评论是裸数组。
- 搜索响应不返回 `highlights`（与 Go 一致），前端继续使用 `excerpt` 兜底。
- 错误统一为 `{"error": "..."}`，状态码和文案与 Go 一致。
- JSON 响应的 `Content-Type` 为 `application/json; charset=utf-8`。
- JWT 使用 `Authorization: Bearer <token>`，只接受 HS256。
- 枚举为小写字符串：`admin|reader`、`published|draft|hidden`、
  `pending|approved|rejected`。
- 时间为 RFC3339 本地偏移格式（如 `2026-11-05T10:00:00+08:00`），不使用
  epoch 数字；与 Go 的 RFC3339Nano 行为一致。
- 方法不匹配返回 Gin 风格的 `404 page not found`（不是 405）；尾斜杠重定向：
  GET 为 301，其他方法为 307。
- CORS 与 Go 一致：`Access-Control-Allow-Origin: *`，任意 OPTIONS 请求返回 204。

### TTS 兼容性细节

- 上游 URL 为 `tts.base_url` 去掉一个末尾斜杠后拼接 `/v1/audio/speech`，请求头
  携带 `Authorization: Bearer <api_key>`。
- 上游 JSON body 按 Go 的字母序输出键，整数值的浮点数不带 `.0`（`1.0` → `1`）。
- 二进制响应为 `application/octet-stream`，带
  `Content-Disposition: attachment; filename=speech.<format>`；SSE 响应为
  `text/event-stream`，带 `Cache-Control: no-cache`，不做缓冲。
- 上游非 200 响应保持 Go 的形状 `{"error":"TTS service error","detail":"<body>"}`，
  body 最多读取 64 KiB；连接失败/超时返回 500
  `{"error":"Failed to call TTS service"}`。
- 共享 `reqwest::Client` 的连接超时为 10 秒；非 SSE 请求总超时 120 秒，SSE
  流不设总超时。

### 搜索行为

- Bleve 的标准分析器把 CJK 文本拆成单字，jieba 不会。因此未加引号的 CJK 片段
  会展开为 `char OR char`，同时匹配 jieba 字段和 `*_uni` 单字字段；54 条语料
  的命中集合与 Go 一致（top-10 重合 100%）。
- `search.fallback_like` 是切换期的安全阀：索引零命中时执行
  `title LIKE ? OR content LIKE ?`，只查已发布、未删除的文章。索引稳定后可改为
  `false`。

## 兼容性说明

完整的逐项说明见 [`docs/compatibility-decisions.md`](docs/compatibility-decisions.md)
与 [`docs/api-differences.md`](docs/api-differences.md)。

### 已批准的修复（Rust 有意改变 Go 行为）

| 项 | 修复内容 |
|---|---|
| Q1 | 搜索索引按 `slug` 维护；更新/删除同步旧 slug，不再留下残留文档 |
| Q2 | 只索引并返回 `published && !is_deleted && deleted_at IS NULL` 的文章 |
| Q2b | 公共文章列表/详情排除 `is_deleted` 行，已删除文章不再泄漏 |
| Q4 | 公共页面使用可选认证：匿名只看 `published`，管理员可见全部 |
| Q6 | 摘要按 200 字节截断时回退到 UTF-8 边界，不产生 `U+FFFD` |
| Q7 | 默认昵称取 `@` 前缀，短邮箱不再 panic/500 |
| Q8 | 头像接受 `.jpg/.jpeg/.png/.gif`（大小写不敏感），`upload.max_size` 生效 |
| Q9 | 通知使用 `buffer_unordered(5)` 并发发送，记录成功/失败数；响应形状不变 |
| Q10 | 回复邮件使用绝对链接，并对昵称/正文/标题/摘要做 HTML 转义 |
| Q11 | TTS 的 `input`/`instructions` 按字符数（4096）校验，不再按字节 |
| Q16 | 启动时创建上传目录，首次上传不再依赖目录预建 |
| Q20 | JWT 只接受 HS256，拒绝 HS384/HS512 |

### 保留的 Go 行为（契约测试锁定）

- **Q3**：公共文章路由没有认证，`?edit=true` 不能预览草稿。
- **Q5**：软删除会把 slug 改成 8 位随机 hex；恢复不还原原 slug，且状态变为
  `draft`；再次 DELETE 仍是软删，只有对 `hidden` 状态 DELETE 才硬删。
- **Q12**：IP 限流每次 `INCR` 后都 `EXPIRE`，固定窗口会被重置。
- **Q13**：页面/订阅分页使用“取数字前缀”的 `parseInt`（`"10abc" → 10`）；
  文章/评论使用 `Atoi`（失败为 0）。
- **Q14**：未 Preload 的 `category`/`author`/`post` 输出零值对象；`tags` 可能
  是 `null` 或 `[]`。
- **Q17**：分类/标签软删除后文章仍引用，Preload 得到零值对象。
- **Q18/Q18b**：评论只返回一层 `children`；回复同时作为顶层项返回。
- **Q19**：软删除的订阅邮箱再次订阅返回 500 `Failed to subscribe`。
- **Q21**：方法不匹配返回 404（无 `Allow` 头）。
- **Q22**：尾斜杠 301（GET）/307（其他方法），带 `Location`。
- **Q23**：panic 返回 500，连接不中断。
- **Q24**：时间输出 Go 风格（本地偏移，`DATETIME(3)` 的 0–3 位小数）。
- **Q25**：`view_count` 自增同时刷新 `updated_at`。
- **Q26**：搜索语法错误返回 500 `Search failed`，不改成 400。
- **Q27**：合成 `user_id=1` 管理员的 `PUT /api/user` 返回 500；头像上传写入
  文件但不更新 `users.avatar`。
- `tag_ids: []` 在更新文章时仍会加载全部存活标签（GORM 关联替换怪癖）。

### 新增的行为差异（不属于计划修复）

| 差异 | 说明 |
|---|---|
| JSON 请求体上限 2 MiB | axum 默认 `DefaultBodyLimit`，超限返回 413；Go 没有全局上限。正常文章远低于此值 |
| 搜索相关性排序 | tantivy 与 Bleve 打分不同：命中集合和 `total` 一致，但同一页内顺序可能不同 |
| TTS 上游请求头 | 不带 `Accept-Encoding: gzip`，User-Agent 不同；不影响 API 响应 |
| 日志格式 | JSON 文件日志 + 控制台；请求字段对齐 Go，但文本格式不同 |

### Markdown 兼容性

Markdown 输出有 25 个 golden 用例锁定，仅两类语义等价差异：comrak 把智能标点
输出为 UTF-8，而 goldmark 输出实体；goldmark 的 `'twas`/`'em` 启发式会给嵌套
的开头单引号一个闭合方向。class 名、表格结构、原始 HTML 和空白均一致；
`![alt](url "")` 通过哨兵保留，不会变成 `<figure>`。

## 性能

测试环境：Apple M5 / 16 GB、macOS、MySQL 8.4、Redis 7.4、`oha 1.16.0`，
每个端点为 3000 请求 × 50 并发。

| 端点 | Go p95 | Rust p95 | 比值 |
|---|---:|---:|---:|
| `GET /api/posts?page=1&page_size=10` | 26.82 ms | 22.37 ms | 0.83× |
| `GET /api/posts/{slug}` | 129.15 ms | 60.53 ms | 0.47× |
| `GET /api/search?keyword=文` | 58.82 ms | 16.66 ms | 0.28× |
| `GET /api/categories`（非目标项） | 12.67 ms | 19.62 ms | 1.55× |
| 峰值 RSS（三项最大） | 44.9 MB | 61.3 MB | 1.37× |

三个计划内端点的 p95 均低于 Go，达到计划阈值，搜索收益最大；峰值内存
61.3 MB，低于 2× 预算。`/api/categories` 的 p95 尾延迟略高，但 p50 更快、
吞吐接近，属于 50 并发下的噪声。

复现：

```bash
./scripts/bench.sh
```

详细方法与结果见 [`docs/performance.md`](docs/performance.md)。

## 部署与回滚

### CI

`.github/workflows/ci.yml` 在每次 push 和 PR 上运行：

```text
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # GitHub runner 自带 Docker
cargo build --locked --release
cargo audit
cargo deny check
```

### Release

`.github/workflows/release.yml` 由 `v*.*.*` tag 触发：

- 构建前先执行 preflight：校验 `SERVERS_CONFIG`，再逐台 SSH 验证连通性，并
  确认 `deploy_path` 存在且部署用户可写；任何一台失败都不会开始构建；
- 使用 `cross` 构建 `x86_64-unknown-linux-musl` 静态二进制；
- 校验 `file` 为静态链接、`readelf -d` 无 `NEEDED` 动态依赖、大小 ≤ 80 MiB；
- 生成恰好三个文件的制品：`smileyan-backend`、`config.yaml`（`mode: prod`）、
  `deploy-remote.sh`，并检查 `.env` 泄漏；
- 发布 `smileyan-backend-prod.tar.gz`；stable tag 会按 `SERVERS_CONFIG`
  自动部署到每台服务器（SSH 端口 `port`，应用端口 `app_port`，默认 8080）。

部署 job 需要的 Repository Variables 与 Secrets 见下文“GitHub 流水线配置”。

### Staging

`.github/workflows/staging.yml` 手动触发，输入要部署的 tag：

- 下载该 tag 的 release tarball；
- 用同一个 `deploy-remote.sh` 部署到 `STAGING_SERVERS_CONFIG`；
- 默认目录 `/opt/smileyan-backend-staging`，端口 8081；
- 部署后自动运行 `scripts/smoke.sh`。

### GitHub 流水线配置

流水线只从 GitHub 读取配置（`Settings → Secrets and variables → Actions`），
不依赖仓库中的 `.env`。构建 job 不需要任何应用密钥；只有部署 job 会消费下面
这些字段。

**Repository Variables**

| 变量 | 使用方 | 说明 |
|---|---|---|
| `SERVERS_CONFIG` | `release.yml` | 生产服务器列表，JSON 数组；stable tag 发布后并行部署到每台 |
| `STAGING_SERVERS_CONFIG` | `staging.yml` | Staging 服务器列表，JSON 数组；手动触发时使用 |

两个变量的元素字段相同：

| 字段 | 必填 | 说明 |
|---|---|---|
| `name` | 是 | 服务器别名，仅用于日志与 job 名称 |
| `addr` | 是 | SSH 主机地址（IP 或域名） |
| `port` | 是 | SSH 端口 |
| `user` | 是 | SSH 用户，需对 `deploy_path` 有写权限 |
| `deploy_path` | 是 | 服务端部署目录，如 `/opt/smileyan-backend` |
| `app_port` | 否 | 应用监听端口，默认 8080；同机 staging 用 8081 |

```json
[
  {
    "name": "prod-1",
    "addr": "1.2.3.4",
    "port": "22",
    "user": "deploy",
    "deploy_path": "/opt/smileyan-backend",
    "app_port": "8080"
  }
]
```

变量必须是合法 JSON 数组，所有字段写成字符串。`SERVERS_CONFIG` 还必须是非空
数组：release workflow 的 `Validate deployment config` job 会在构建前校验，
空值、非法 JSON 或缺少必填字段都会直接失败，不会开始编译。
`STAGING_SERVERS_CONFIG` 可省略 `app_port`（回退到 workflow 输入的 8081）。

**Repository Secrets**

| Secret | 必填 | 说明 |
|---|---|---|
| `SMILEYAN_BACKEND_DB_HOST` | 是 | `database.host` |
| `SMILEYAN_BACKEND_DB_USER` | 是 | `database.user` |
| `SMILEYAN_BACKEND_DB_PASSWORD` | 是 | `database.password`，不会打印到日志 |
| `SMILEYAN_BACKEND_DB_NAME` | 是 | `database.dbname` |
| `SMILEYAN_BACKEND_REDIS_HOST` | 是 | `redis.host` |
| `SMILEYAN_BACKEND_REDIS_USERNAME` | 是 | `redis.username`；部署脚本要求非空，无用户名时填 `default` |
| `SMILEYAN_BACKEND_REDIS_PASSWORD` | 是 | `redis.password`，不会打印到日志 |
| `SMILEYAN_BACKEND_EMAIL_PASSWORD` | 是 | `email.password` |
| `SMILEYAN_BACKEND_JWT_SECRET` | 是 | `jwt.secret`，与 Go 版共用 |
| `DEPLOY_SSH_KEY` | 是 | SSH 私钥；对应公钥需加入每台服务器的 `authorized_keys` |
| `SMILEYAN_BACKEND_TTS_BASE_URL` | 否 | `tts.base_url` |
| `SMILEYAN_BACKEND_TTS_API_KEY` | 否 | `tts.api_key` |
| `SMILEYAN_BACKEND_TTS_DEFAULT_MODEL` | 否 | `tts.default_model` |

`GITHUB_TOKEN` 由 Actions 自动注入，无需手动创建；release job 依赖它创建
Release，deploy job 依赖它下载 tarball。任一必填 Secret 为空时，部署 job 会在
第一步列出缺失项并失败，不会继续 SSH。TTS 三项为空时不会写入服务端 `.env`。

### 部署脚本

`scripts/deploy-remote.sh` 在目标机上执行：

1. 校验必填环境变量，`APP_PORT` 必须是数字；
2. 原子写入 `.env`（权限 0600），TTS 变量仅在非空时写入；
3. 按 `logs/app.pid` 停止旧进程（校验命令行），没有 PID 文件时回退旧
   `stop.sh`；
4. **解压前**备份 `smileyan-backend` → `.prev`、`config.yaml` → `.prev`；
5. 解压三件套，删除部署目录中的 `deploy-remote.sh` 与 tarball；
6. 按 `APP_PORT` 改写 `config.yaml` 的 `server.port` 并校验；
7. 生成 `start.sh`、`stop.sh`、`restart.sh`、`rollback.sh`；
8. 启动并轮询 `http://127.0.0.1:<APP_PORT>/api/categories`（30 秒），失败时
   打印最后 50 行日志并提示回滚。

生成的 `stop.sh`/`rollback.sh` 只处理本目录 PID 和本实例端口，**不会**
`pkill -f smileyan-backend`，因此 staging 8081 与生产 8080 可以同机共存。

### 回滚

```bash
cd /opt/smileyan-backend
./rollback.sh
```

回滚过程：停止当前进程 → 把失败二进制保留为 `smileyan-backend.failed` →
恢复 `smileyan-backend.prev` 与 `config.yaml.prev` → 启动并健康检查 →
打印恢复后的 sha256。`.prev` 在回滚后仍保留，可重复尝试；重新部署会刷新它。

回滚触发条件：

- 5xx 比例 > 1% 持续 5 分钟；
- p95 超过 Go 基线 2 倍持续 10 分钟；
- 登录、发文、评论任一核心链路失败；
- Markdown 渲染或搜索出现用户可见异常。

### 冒烟与回滚演练

```bash
# 部署后冒烟（第二个参数可选 JWT；无 token 时跳过认证检查）
./scripts/smoke.sh http://127.0.0.1:8081 "$SMILEYAN_BLOG_TOKEN"

# 回滚演练（会停止并重启目标实例，必须显式 --yes）
./scripts/rollback_drill.sh \
  --deploy-path /opt/smileyan-backend-staging \
  --port 8081 \
  --token "$SMILEYAN_BLOG_TOKEN" \
  --keyword 测试 \
  --yes
```

`smoke.sh` 使用纯 curl/grep，可在裸服务器上运行，覆盖健康检查、公共列表、
搜索、静态文件 404 以及可选的认证接口。`rollback_drill.sh` 验证回滚前后
sha256、旧 JWT、数据与搜索索引，并打印耗时；演练后重新部署同一 tag 即可回到
新版本。

完整的构建、staging、生产切换、灰度、回滚和 7 天观察流程见
[`docs/deployment.md`](docs/deployment.md)。

## 项目结构

```text
Cargo.toml / Cargo.lock / rust-toolchain.toml
config.yaml                     # 与 Go 基线相同的字段（新增 site_url/tts/search）
.env.example                    # 环境变量模板
migrations/0001_baseline.sql    # CREATE TABLE IF NOT EXISTS 基线
src/
  main.rs                       # 启动顺序 + 优雅退出
  app.rs                        # Router、CORS、404/301/307、JSON charset
  bootstrap.rs                  # migrations、默认管理员、上传目录、搜索重建
  config.rs                     # YAML + 环境变量覆盖 + 启动校验
  error.rs                      # AppError → {"error":"..."}
  logging.rs                    # 控制台 + 按日 JSON 文件
  state.rs                      # AppState（MySQL、Redis、Mailer、HTTP、时区）
  bin/genjwt.rs                 # cargo run --bin genjwt
  bin/rebuild_html.rs           # cargo run --bin rebuild_html
  handlers/                     # auth/users/posts/categories/tags/pages/comments/
                                # subscriptions/search/tts/uploads/hello
  middleware/                   # auth/admin/optional、客户端 IP、请求日志
  models/                       # DB 行、响应 DTO、Go 时间序列化
  repo/                         # 所有 SQL（显式软删除过滤）
  services/                     # markdown、search、email、token、限流、slug、参数解析
tests/
  common/mod.rs                 # testcontainers + 种子数据 + RecordingMailer
  http_contract.rs              # Phase 1 HTTP 契约
  auth_contract.rs              # Phase 2 认证/用户 + JWT 互通
  content_contract.rs           # Phase 3 内容域
  comments_contract.rs          # Phase 4 评论
  subscriptions_contract.rs     # Phase 4 订阅 + 通知
  search_contract.rs            # Phase 5 搜索语料/生命周期/兜底
  tts_contract.rs               # Phase 6 TTS 校验/流式/错误
  contract_full.rs              # Phase 7 全量 193 夹具重放
  fixtures/                     # Phase 0 冻结夹具（不要手改）
bin/
  start stop restart dev        # 本地进程管理（按配置端口）
  build-and-package.sh          # cross/zigbuild/host 打包
scripts/                        # capture_*、bench、deploy-remote、smoke、rollback_drill
docs/                           # 决策、前端核查、API 差异、性能、安全、部署
deny.toml                       # cargo-deny 许可证/公告/来源策略
```

## 文档索引

| 文档 | 内容 |
|---|---|
| [`docs/compatibility-decisions.md`](docs/compatibility-decisions.md) | Q1–Q27 的修复/保留决策、实测证据、每阶段记录 |
| [`docs/frontend-api-usage.md`](docs/frontend-api-usage.md) | 前端真实调用的路径、字段与必须保留的响应契约 |
| [`docs/api-differences.md`](docs/api-differences.md) | Rust 与 Go 的已批准差异、保留怪癖、新增差异 |
| [`docs/performance.md`](docs/performance.md) | Go vs Rust 压测方法、明细与复现 |
| [`docs/security-checklist.md`](docs/security-checklist.md) | 认证、SQL、上传、TTS、CORS、依赖审计与剩余风险 |
| [`docs/deployment.md`](docs/deployment.md) | 构建、CI、staging、回滚演练、7 天观察 |
| [`AGENTS.md`](AGENTS.md) | 面向 AI 编码助手的项目约束与常用命令 |

## 契约夹具（Phase 0）

`tests/fixtures/` 是冻结的 Go 基线输出，**不要手工修改**。重新生成只能针对新的
Go 基线 tag，并在提交中说明原因。

| 夹具 | 数量 | 内容 |
|---|---:|---|
| `contract/cases/` | 172 | HTTP 请求/响应记录（状态、头、JSON、二进制摘要） |
| `contract/emails/` | 8 | MailHog 解码后的邮件（主题/正文已归一化） |
| `contract/notes/` | 13 | Redis 快照、TTS 上游请求、id、时间格式样本 |
| `markdown/corpus/` + `golden/` | 25 + 25 | Markdown 输入与 Go 渲染结果 |
| `search/` | 54 | 搜索查询与采集结果 |

归一化规则：时间戳 → `<timestamp>`、JWT → `<jwt>`、32 位 hex token →
`<token>`、8 位随机 slug → `<random-slug>`、6 位验证码 → `<code>`；
`content-length` 不采集。实现见 `scripts/lib/capture_common.py`。

重新生成：

```bash
cp .env.capture.example .env.capture

# Markdown：不需要 Docker，可直接校验已提交的 golden
./scripts/capture_markdown.sh --check

# HTTP 契约：启动 Docker、执行 seed、运行 Go 服务、TTS mock 与 MailHog
./scripts/capture_contract.sh

# 仅搜索语料
./scripts/capture_search.sh
```

`BACKEND_DIR` 可在 `.env.capture` 或环境中覆盖。`CAPTURE_KEEP=1` 会在采集后
保留 Docker 栈，便于调试；清理使用
`docker compose -f tests/fixtures/docker-compose.yml down -v`。

## 已知限制与后续项

- **上传文件类型**：目前只校验扩展名和大小，未校验文件魔数；后续可用 `infer`
  检查 PNG/JPEG/GIF 文件头。
- **可信代理**：`X-Forwarded-For` 完全信任。生产在反向代理后面时应增加
  `trusted_proxies`，只接受来自代理的 XFF（当前保持 Go 行为）。
- **限流窗口**：IP 限流每次 `INCR` 后都会 `EXPIRE`，窗口会被重置（Q12）；
  后续可用 Lua 实现原子固定窗口。
- **合成管理员**：任何合法签名且 `user_id=1` 的 token 都会被当作管理员
  （见 `docs/security-checklist.md`）。切换稳定后建议评审移除，并同步更换长期
  token。
- **JSON 请求体**：默认上限 2 MiB；如有更大的 Markdown 正文需求，可调整
  `DefaultBodyLimit` 或改用附件上传。
- **搜索排序**：tantivy 与 Bleve 的相关性打分不同，命中集合与 `total` 一致但
  页内顺序可能不同。
- **生产切换**：musl 构建由 CI 的 cross 执行；真实 staging/生产部署与
  7 天观察需要服务器权限，按 [`docs/deployment.md`](docs/deployment.md)
  执行。
