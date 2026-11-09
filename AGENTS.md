# AGENTS.md

面向 AI 编码助手的项目说明。目标是**行为等价地重写** Go/Gin 后端
（`../smileyan-backend`，基线 tag `go-baseline-2026-11`），不是趁机重构 API。

## 项目结构

```
src/
  main.rs / app.rs       启动顺序、Router、CORS、404/301、JSON charset
  bootstrap.rs           migrations、默认管理员、上传目录、搜索重建
  config.rs              YAML + SMILEYAN_BACKEND_* 覆盖 + 启动校验
  state.rs               MySQL pool、Redis manager、Mailer、reqwest、时区
  error.rs               AppError -> {"error":"..."}
  handlers/              每个端点一个文件（auth/users/posts/.../tts/uploads）
  repo/                  所有 SQL；显式 deleted_at / is_deleted / Unscoped
  services/              markdown、search(tantivy)、email、token、限流、slug
  middleware/            auth/admin/optional、请求日志、客户端 IP
  models/                DB 行、响应 DTO、Go 时间序列化
tests/
  common/mod.rs          testcontainers MySQL/Redis + seed + RecordingMailer
  *_contract.rs          各阶段契约测试
  contract_full.rs       全部 172 HTTP + 8 邮件 + 13 note 的重放
  fixtures/              Phase 0 冻结夹具（不要手改，用 scripts/capture_*.sh 重生成）
migrations/0001_baseline.sql
bin/                     start/stop/restart/dev、build-and-package.sh
deploy/nginx/            灰度 location 块 + 切换/回滚/巡检手册
scripts/                 capture_*（Phase 0）、bench.sh（Phase 7）、
                         deploy-remote.sh、smoke.sh、rollback_drill.sh（Phase 8）
docs/                    决策表、前端核查、API 差异、性能、安全、部署
```

## 常用命令

```bash
cargo run                                  # 本地 8080（需 MySQL/Redis/SMTP）
./bin/start / ./bin/stop / ./bin/restart   # 本地进程（按配置端口）
./bin/dev                                  # cargo watch（未安装则单次运行）
./bin/build-and-package.sh all             # 三件套包（cross→zigbuild→host）
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets                   # 需要 Docker
cargo build --locked --release
cargo audit && cargo deny check
./scripts/bench.sh                         # Go vs Rust 压测
./scripts/smoke.sh http://127.0.0.1:8081   # 部署后冒烟（可选第 2 参 JWT）
./scripts/rollback_drill.sh --deploy-path /opt/smileyan-backend-staging \
  --port 8081 --token "$SMILEYAN_BLOG_TOKEN" --keyword 测试 --yes
cargo run --bin genjwt                     # 生成 admin/dev token
cargo run --bin rebuild_html               # 改 Markdown 管线后重建 html_content
```

集成测试会启动一次性 MySQL 8.4 + Redis 7.4；harness 已等待真实
`SELECT 1`/`PING`、固定使用 `127.0.0.1`（`localhost` 的 IPv6 解析会导致间歇
失败），并用跨进程文件锁串行化测试二进制，容器映射固定端口
23306/26379（可用 `TEST_MYSQL_PORT`/`TEST_REDIS_PORT` 覆盖）。不要把端口改回
Docker 随机高位端口：它会与 TIME_WAIT 套接字冲突，导致连接挂起。

## 必须保持的契约

- 路径/方法/认证层级、状态码、错误文案、响应 JSON 形状（裸对象/数组，无
  `{data}` 包装）、`application/json; charset=utf-8`。
- JWT HS256、claims 字段、`SMILEYAN_BACKEND_JWT_SECRET`；Go/Rust token 双向可用。
- 数据库 schema、软删除语义（`deleted_at` + posts 的 `is_deleted`）、slug 规则。
- Redis key/TTL：`verification_code:*`、`rate_limit:email:*`、`rate_limit:ip:*`。
- 环境变量名与 `config.yaml` 字段；`database.port`/`redis.port` 只读 YAML。
- 时间输出 Go RFC3339Nano 风格（`+08:00`、DATETIME(3) 0–3 位小数），用
  `models/go_time.rs`，不要改成 UTC。
- Markdown 的 class 契约在 `services/markdown.rs`；改动后必须跑 25 个 golden。

## 已批准与必须保留的差异

**修复（已在 `tests/contract_full.rs` 显式调整）**：Q1/Q2（搜索按 slug、只索引
published）、Q2b（公共文章排除 is_deleted）、Q4（公共页面只返回 published，
管理员可选认证）、Q6（摘要 UTF-8 边界）、Q7（昵称取 @ 前缀）、Q8（.jpeg/大小写）、
Q9（通知并发）、Q10（邮件绝对链接）、Q11（TTS 字符数）、Q16（上传目录）、
Q20（只接受 HS256）。

**保留的 Go 怪癖**：Q3、Q5、Q12、Q13、Q14、Q17、Q18/Q18b、Q19、Q21、Q22、
Q23、Q24、Q25、Q26、Q27，以及 `tag_ids: []` 加载全部标签。详见
`docs/api-differences.md`。

新增行为差异：JSON body 上限 2 MiB（axum 默认）；搜索相关性顺序与 Bleve
不同但命中集合/`total` 一致。改契约前先更新文档与 `contract_full.rs` 的
调整表，否则测试会以夹具名报错。

## 修改夹具

不要手改 `tests/fixtures/`。Phase 0 采集脚本可重复生成：

```bash
./scripts/capture_markdown.sh --check
./scripts/capture_contract.sh
./scripts/capture_search.sh
```

只有 Go 基线换了 tag 才允许重新生成，并在提交里说明。

## 部署约束（Phase 8）

- `scripts/deploy-remote.sh` 生成的 `stop.sh`/`rollback.sh` **只能**按本目录
  PID 和本实例端口停止进程。不要加 `pkill -f smileyan-backend`：staging
  8081 与生产 8080 可能同机运行。
- 部署前必须把当前二进制备份为 `smileyan-backend.prev`，并生成
  `rollback.sh`；`rollback.sh` 要保留失败二进制 `smileyan-backend.failed`。
- release 制品只能有三个文件：`smileyan-backend`、`config.yaml`、
  `deploy-remote.sh`；`config.yaml` 为 `mode: prod`，staging 的 8081 由
  `APP_PORT` 在服务端改写。
- TTS 三个环境变量只在非空时写入 `.env`，不要把空 key 或 key 值写进日志。
- 灰度期间 `/api/search` 固定走 Go，写请求（非 GET/HEAD/OPTIONS）也走 Go；
  两个进程各自维护内存索引。详见 `deploy/nginx/README.md`。
- 回滚不需要动数据库/Redis；migrations 只做 `CREATE TABLE IF NOT EXISTS`。

## 文档索引

- `docs/compatibility-decisions.md` — Q1–Q27 决策 + 每阶段实施记录
- `docs/frontend-api-usage.md` — 前端真实读取的路径/字段
- `docs/api-differences.md` — Rust vs Go 的批准差异
- `docs/performance.md` — Phase 7 压测方法与结果
- `docs/security-checklist.md` — 安全检查项与剩余风险
- `docs/deployment.md` — 构建、CI release/staging、灰度、回滚演练、7 天观察
