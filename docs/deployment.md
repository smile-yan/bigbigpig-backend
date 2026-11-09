# 部署与回滚（Phase 8）

> Phase 8 交付：release workflow（musl 三件套 + 静态/大小检查 + 自动部署）、
> staging workflow（8081）、`deploy-remote.sh`（`.prev` 备份、`rollback.sh`、
> TTS 变量、端口隔离）、本地 `bin/` 脚本、`scripts/smoke.sh`、
> `scripts/rollback_drill.sh`。
>
> 生产切换、7 天观察需要真实服务器与运维授权，按本文步骤执行；脚本与配置
> 已在本机用真实二进制 + MySQL/Redis 完成部署/回滚演练（见 §8）。

## 1. 构建

目标：`x86_64-unknown-linux-musl` 静态二进制，二进制名保持
`smileyan-backend`，部署脚本无需改名。

```bash
# 方案 A（推荐）：cross
cargo install cross --git https://github.com/cross-rs/cross
cross build --locked --release --target x86_64-unknown-linux-musl

# 方案 B：cargo-zigbuild（本机已装 zig 时）
cargo install cargo-zigbuild
cargo zigbuild --locked --release --target x86_64-unknown-linux-musl

# 本地打包（自动选择 cross → zigbuild → host，输出三个环境包）
./bin/build-and-package.sh all
```

验收：

```bash
file target/x86_64-unknown-linux-musl/release/smileyan-backend
# ... statically linked
readelf -d target/x86_64-unknown-linux-musl/release/smileyan-backend | grep NEEDED
# 无输出：没有动态依赖
```

不要用 `ldd` 判断：Ubuntu 24.04 的 `ldd` 对 static-pie 二进制会打印
`statically linked` 并以 0 退出，看起来像“动态链接”。`DT_NEEDED` 才是可靠信号。

CI 的 release workflow 会执行同样的检查，并额外限制二进制 ≤ 80 MiB。

依赖约束（Phase 1 已验证，Phase 7/8 复核）：

- `jsonwebtoken` 使用 `rust_crypto`，不启用 `aws_lc_rs`。
- `reqwest` 使用 `rustls-no-provider` + 进程内 ring provider。
- `lettre` 使用 `tokio1-rustls` + `ring` + `webpki-roots`。
- `comrak` 已关闭默认 feature（`cli`、`syntect-onig`），不引入 onig C 库。
- `cargo tree` 中不应出现 `openssl-sys`、`aws-lc-sys`、`onig_sys`。

## 2. 制品

release workflow 生成的三件套：

```
release/smileyan-backend      # musl 静态二进制
release/config.yaml           # mode: prod（server.port 8080）
release/deploy-remote.sh      # 服务端部署脚本
```

`release/` 只允许这三个文件；workflow 会检查多余文件与 `.env` 泄漏，并打出
`smileyan-backend-prod.tar.gz`。staging 复用同一制品，由部署脚本把
`server.port` 改成 8081。

本地 `output/` 也会生成 `smileyan-backend-{dev,prod,staging}.tar.gz`，内容
相同；`dev` 保留 `mode: debug`。

## 3. 配置

### 3.1 环境变量（名称不变）

| 变量 | 配置项 | 必填 | 说明 |
|---|---|---|---|
| `SMILEYAN_BACKEND_DB_HOST` | `database.host` | 是 | |
| `SMILEYAN_BACKEND_DB_USER` | `database.user` | 是 | |
| `SMILEYAN_BACKEND_DB_PASSWORD` | `database.password` | 是 | 不打印 |
| `SMILEYAN_BACKEND_DB_NAME` | `database.dbname` | 是 | |
| `SMILEYAN_BACKEND_REDIS_HOST` | `redis.host` | 是 | |
| `SMILEYAN_BACKEND_REDIS_USERNAME` | `redis.username` | 线上必填 | 本地可空；deploy 脚本按 Go workflow 要求非空 |
| `SMILEYAN_BACKEND_REDIS_PASSWORD` | `redis.password` | 是 | 不打印 |
| `SMILEYAN_BACKEND_EMAIL_PASSWORD` | `email.password` | 是 | 不打印 |
| `SMILEYAN_BACKEND_JWT_SECRET` | `jwt.secret` | 是 | 与 Go 相同，旧 token 继续有效 |
| `SMILEYAN_BACKEND_TTS_BASE_URL` | `tts.base_url` | 否 | 留空则 TTS 返回 500 |
| `SMILEYAN_BACKEND_TTS_API_KEY` | `tts.api_key` | 否 | 不打印 |
| `SMILEYAN_BACKEND_TTS_DEFAULT_MODEL` | `tts.default_model` | 否 | |
| `SMILEYAN_BACKEND_DEV_TOKEN` | 运行时读取 | 否 | 仅本地，生产不设置 |

`database.port`、`redis.port/db`、`email.host/port/username/use_ssl`、
`jwt.expire_hours`、`upload.*`、`admin_emails` 仍只从 `config.yaml` 读取。

### 3.2 必改项

- `server.port`：生产 8080；staging 8081（部署脚本自动改写）。
- `server.timezone`：默认 `Asia/Shanghai`；保持与生产机器一致。
- `database` / `redis`：host、port、db。
- `email`：生产 `smtp.exmail.qq.com:465` + `use_ssl: true`。
- `upload.path` / `upload.avatar_path`：确认运行用户有写权限。
- `site_url`：邮件绝对链接前缀，默认 `https://smileyan.cn`。
- `search.fallback_like`：切换期保持 `true`，稳定后可关闭。

## 4. CI

### 4.1 `.github/workflows/release.yml`

- tag `v*.*.*` 触发；release job 不需要任何应用 Secret（Rust 在运行时读配置）。
- `Validate deployment config`：无 matrix，先打印并校验 `SERVERS_CONFIG`
  （非空 JSON 数组、`name/addr/port/user/deploy_path` 必填）与
  `DEPLOY_SSH_KEY`；失败则整个流水线在构建前终止。
- `Preflight <server>`：按 `SERVERS_CONFIG` 逐台 SSH（`fail-fast: false`），
  验证密钥登录成功，并检查 `deploy_path` 存在、当前用户可写可进入。
  构建 job `needs: preflight`，任一机器失败都不会开始 cross 构建。
- `cross build --locked --release --target x86_64-unknown-linux-musl`。
- 静态链接检查（`file` + `readelf -d` 的 `DT_NEEDED`）、80 MiB 大小上限、三件套清单检查、
  `.env` 泄漏检查。
- 发布 `smileyan-backend-prod.tar.gz`；stable tag 自动部署到
  `SERVERS_CONFIG` 中的每台机器（SSH 端口 `port`，应用端口 `app_port`，
  缺省 8080）；deploy job 依赖 `preflight` 与 `release`。
- deploy job 先把 release tarball 与 `deploy-remote.sh` 合并压缩为
  `smileyan-backend-deploy.tar.gz`，日志打印压缩后大小（MB），再用 `pv`
  流式上传并每 30 秒输出一次百分比进度；服务端解包后执行 `deploy-remote.sh`。
- 部署前检查 9 个应用 Secret + `DEPLOY_SSH_KEY` 非空；TTS 三个 Secret
  可选，空值不会写进 `.env`。
- 部署后检查日志、进程、`:app_port` 监听、`.prev`/`rollback.sh` 是否存在，
  并用 `/api/categories` 做健康检查。

`SERVERS_CONFIG` 示例：

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

### 4.2 `.github/workflows/staging.yml`

手动 `workflow_dispatch`，输入要部署的 tag；下载该 tag 的 release tarball，
用同一 `deploy-remote.sh` 部署到 `STAGING_SERVERS_CONFIG`，默认
`/opt/smileyan-backend-staging` + 8081，然后上传并运行 `scripts/smoke.sh`。

```json
[
  {
    "name": "staging-1",
    "addr": "1.2.3.4",
    "port": "22",
    "user": "deploy",
    "deploy_path": "/opt/smileyan-backend-staging"
  }
]
```

## 5. 部署

### 5.1 启动顺序（与 Go 一致）

1. 读取 `.env`（可选）与 `config.yaml`，应用环境变量覆盖。
2. 初始化日志：控制台 + `logs/YYYY-MM-DD.log`（JSON）。
3. 校验必填配置（JWT secret、DB/Redis 等）；缺失直接退出。
4. 连接 MySQL；连接 Redis 并 `PING`。
5. 执行 migrations（`RUN_MIGRATIONS=false` 可关闭；baseline 为
   `CREATE TABLE IF NOT EXISTS`，首次在旧库执行不会破坏表）。
6. users 表为空时插入默认管理员 `root@smileyan.cn`。
7. 重建 tantivy 内存索引（只索引 published/未删除文章）。
8. 创建 `uploads/`、`uploads/avatars/` 目录。
9. 监听 `0.0.0.0:<port>`；SIGTERM/SIGINT 优雅退出。

### 5.2 `deploy-remote.sh` 行为

1. 校验全部必填环境变量；`APP_PORT` 必须是数字。
2. 原子写入 `.env`（0600）；TTS 变量仅在非空时写入。
3. 先按 `logs/app.pid` 停止旧进程（校验命令行包含 `smileyan-backend`，
   避免误杀复用 PID）；没有 PID 文件时回退旧 `stop.sh`。
4. **解压前**备份：
   - `smileyan-backend` → `smileyan-backend.prev`
   - `config.yaml` → `config.yaml.prev`
5. 解压三件套，删除部署目录里的 `deploy-remote.sh` 与 tarball。
6. 按 `APP_PORT` 改写 `config.yaml` 的 `server.port` 并校验结果。
7. 生成 `start.sh`/`stop.sh`/`restart.sh`/`rollback.sh`：
   - `stop.sh` 只处理本目录 PID 与本端口的监听进程，绝不 `pkill -f`；
     因此 staging 8081 与生产 8080 可以同机共存。
8. 启动并轮询 `http://127.0.0.1:<APP_PORT>/api/categories`（30 s）；
   失败时打印最后 50 行日志并提示 `rollback.sh`，以非 0 退出。

### 5.3 staging（8081）

1. 在 `STAGING_SERVERS_CONFIG` 配好 staging 主机（可复用生产 DB/Redis，
   或指向 staging 实例）。
2. Actions → `Deploy staging` → Run workflow，填入 tag（可用 rc）。
3. 流程会自动执行：
   ```bash
   curl -fsS http://127.0.0.1:8081/api/hello
   bash /tmp/smoke-staging.sh http://127.0.0.1:8081
   ```
4. 手工补一遍写链路：登录（MailHog/真实 SMTP）、发文/编辑、评论、搜索、
   TTS（若启用）。
5. 保留 `.prev` 与 `.env`，供回滚使用。

### 5.4 生产（维护窗口，首次推荐）

1. 选低峰期，确认 staging 回归通过。
2. 运行 release workflow（打 stable tag）或手动执行：
   ```bash
   scp smileyan-backend-prod.tar.gz deploy@server:/tmp/
   scp deploy-remote.sh deploy@server:/tmp/
   ssh deploy@server "DEPLOY_PATH=/opt/smileyan-backend RELEASE_TAG=vX.Y.Z \
     APP_PORT=8080 TARBALL_PATH=/tmp/smileyan-backend-prod.tar.gz \
     S_DB_HOST=... ... bash /tmp/deploy-remote.sh"
   ```
3. 脚本会停旧进程、备份 `.prev`、启动 Rust 并健康检查。
4. 确认前端登录/发文/评论/搜索正常。
5. 观察 30 分钟，异常执行 §6 回滚。

## 6. 回滚

触发条件（计划 §9.4）：

- 5xx 率 > 1% 持续 5 分钟；
- p95 > Go × 2 持续 10 分钟；
- 登录/发文/评论任一核心链路失败；
- Markdown 渲染或搜索出现用户可见异常。

### 6.1 二进制回滚（Rust 占用 8080）

```bash
cd /opt/smileyan-backend
./rollback.sh
```

`rollback.sh`：

1. `stop.sh`（仅本端口/本 PID）；
2. 当前二进制另存为 `smileyan-backend.failed`（保留现场）；
3. `smileyan-backend.prev` → `smileyan-backend`；
4. `config.yaml.prev` → `config.yaml`（若存在）；
5. `start.sh`，轮询 `/api/categories` 30 s，打印恢复后的二进制 sha256。

`.prev` 在回滚后仍保留，可重复尝试；重新部署会刷新它。

### 6.2 回滚演练

```bash
./scripts/rollback_drill.sh \
  --deploy-path /opt/smileyan-backend-staging \
  --port 8081 \
  --token "$SMILEYAN_BLOG_TOKEN" \
  --keyword 测试 \
  --yes
```

检查项：

- 回滚前后二进制 sha256 与 `.prev` 一致；
- `/api/hello`、`/api/posts` 正常；
- 旧 JWT 可访问 `/api/user`；
- 搜索关键词有命中（进程重启后索引已重建）；
- 打印耗时。

演练后重新部署同一 tag 即回滚到新版本。

## 7. 观察（7 天）

- 健康检查：`curl -fsS http://127.0.0.1:8080/api/categories` 200。
- 日志：`logs/YYYY-MM-DD.log` 中 5xx、`Failed to`、panic；请求字段
  `method/path/ip/status`。
- 进程/内存：`ps -o pid,rss,etime,args -C smileyan-backend`；tantivy 索引
  常驻，Phase 7 实测空载约 22 MB、50 并发峰值约 61 MB。
- 邮件：验证码、订阅确认、回复/通知；SMTP 失败日志。
- TTS：上游错误日志（body 截断 64 KiB）。
- 每天手工走一遍登录、发文、评论、搜索、TTS。

观察期内保留 Go 二进制、`.env`、`go-baseline-2026-11` tag。

## 8. Phase 8 执行记录（2026-11-05）

| 项 | 结果 |
|---|---|
| 分支 | `phase8/deploy-cutover` |
| release workflow | `.github/workflows/release.yml`：cross musl、静态/`DT_NEEDED` 检查、80 MiB 上限、三件套清单、tag 部署、TTS Secret 转发、`app_port` 支持 |
| staging workflow | `.github/workflows/staging.yml`：手动 tag、`STAGING_SERVERS_CONFIG`、8081、部署后 smoke |
| 部署脚本 | `scripts/deploy-remote.sh`：原子 `.env`（TTS 可选）、PID/端口隔离停止、`.prev`/`config.yaml.prev` 备份、`rollback.sh`、端口改写、30 s 健康检查 |
| 本地脚本 | `bin/start|stop|restart|dev|build-and-package.sh`；打包输出三件套（dev/prod/staging），本地验证 staging 配置为 8081 |
| 冒烟/演练 | `scripts/smoke.sh` 13 项全过；`scripts/rollback_drill.sh` 在真实二进制 + MySQL 8.4/Redis 7.4 上执行：部署 v1 → 部署 v2 → 回滚，2 s 完成，sha256 恢复、旧 JWT 200、搜索命中 1 条 |
| 门禁 | `cargo fmt --all --check`、`cargo clippy --all-targets -D warnings`、`cargo test --all-targets`、`cargo build --locked --release`、`cargo audit` 全部通过 |
| 未在本机执行 | musl 交叉编译（本机网络受限，未安装 musl target/cross；由 CI 的 cross 执行）；真实 staging/生产部署、7 天观察需服务器权限 |
| 待运维执行 | 打 stable tag 触发 release；`Deploy staging` 验证 8081；§6 生产回滚演练；§7 观察 7 天；Go 仓库归档（保留 `go-baseline-2026-11`） |

## 9. Go 仓库归档

观察结束后：

```bash
# Go 仓库只保留回滚 tag，不再接受功能提交
git -C ../smileyan-backend tag go-baseline-2026-11   # 已存在
# 在仓库设置中 Archive，或把 main 冻结；不要删除 tag
```

Rust 仓库成为唯一活跃实现；`docs/rust-rewrite-plan.md` 的 Phase 8 记录
指向本节。
