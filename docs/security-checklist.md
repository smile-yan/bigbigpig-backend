# 安全 checklist（Phase 7）

> 日期：2026-11-05
> 范围：Rust 重写后的 HTTP 服务、MySQL/Redis、SMTP、TTS 代理、上传/静态文件、依赖链
> 对应计划 §7.6；「保留」表示与 Go 基线一致、「增强」表示 Rust 侧更严格。

## 结论

| 检查项 | 状态 | 证据 / 说明 |
|---|---|---|
| JWT 只接受 HS256 | ✅ 增强 | `services/token.rs` 使用 `Validation::new(Algorithm::HS256)`；HS384/512 token 返回 401（Q20，夹具 `admin_route_hs512_token`） |
| JWT 密钥非空 | ✅ | `config.rs` 启动校验 `jwt.secret` 非空，缺失即退出 |
| 过期/签名错误/缺 header | ✅ | 401 文案与 Go 一致：`Authorization header is required`、`Invalid authorization header format`、`Invalid or expired token`；`leeway=0`、校验 `nbf`、要求 `exp` |
| dev token 仅本地 | ✅ 保留 | 仅当 `SMILEYAN_BACKEND_DEV_TOKEN` 非空且签名/claim 匹配时生效；生产不设置该变量 |
| `user_id == 1` 合成管理员 | ⚠️ 保留 | 与 Go 一致的安全怪癖：任何合法签名且 `user_id=1` 的 token 会被当作管理员；切换后建议评审移除（需同步换 token） |
| SQL 参数绑定 | ✅ | 全部查询使用 `sqlx::query`/`QueryBuilder::push_bind`；未发现字符串拼接 SQL |
| LIKE 通配符转义 | ✅ 增强 | `repo/posts.rs::like_pattern` 转义 `\`、`%`、`_`；Go 基线未转义 |
| 上传大小限制 | ✅ 修复 | `upload.max_size`（默认 5 MB）在读取 multipart 时逐块检查，超限 400 `File too large` |
| 上传扩展名白名单 | ✅ 修复 | `jpg/jpeg/png/gif`，大小写不敏感；Go 的“最后 4 个字符”逻辑已修正（Q8） |
| 路径穿越/控制字符 | ✅ 修复 | `sanitize_filename` 取 `file_name` 并过滤控制字符；文件写入 `avatar_path` |
| 上传魔数校验 | ⚠️ 未做 | 仅扩展名+大小；`infer` 校验留作后续安全项（Go 也没有）。见「后续项」 |
| 静态文件目录列表 | ✅ | `ServeDir` 不列目录；只挂载 `/uploads` |
| TTS 上游 URL 来源 | ✅ | 只来自 `tts.base_url` 配置，不接受用户输入 |
| TTS 错误体截断 | ✅ 增强 | 上游非 200 最多读取 64 KiB |
| TTS 超时 | ✅ 增强 | 共享 `reqwest::Client`：connect 10 s；非 SSE 总超时 120 s；SSE 不设总超时 |
| TTS API key 日志 | ✅ | 只记录“是否配置”，不记录 key；错误日志不包含 Authorization |
| CORS | ✅ 保留 | `Allow-Origin: *`、方法/头与 Gin 一致、不设置 `Access-Control-Allow-Credentials` |
| 请求体上限 | ✅ 增强 | axum 默认 2 MiB（JSON/DTO 路由）；上传路由禁用全局限制后按 `upload.max_size` 手工限制。Go 无上限，见「已知差异」 |
| 验证码/限流 Redis key | ✅ 保留 | `verification_code:<email>` 600 s、`rate_limit:email:<email>` 600 s、`rate_limit:ip:<ip>` INCR+EXPIRE 3600 s；与 Go 完全一致 |
| 管理员邮箱限流绕过 | ✅ 保留 | `admin_emails` 大小写敏感，与 Go 一致 |
| 登录失败限流 | ⚠️ 保留 | 与 Go 一致：只对 `send-code` 限流，登录验证码错误不限流（Q12 固定窗口缺陷也保留） |
| 客户端 IP | ⚠️ 保留 | 依次取 `X-Forwarded-For` 第一项、`X-Real-IP`、socket；未配置可信代理，XFF 可被伪造（内网/反代场景可接受，见后续项） |
| 日志不含敏感信息 | ✅ | 请求日志仅 `method/path/ip/status`；JWT、验证码、密码、TTS key 不写日志 |
| panic 不泄露 | ✅ | `CatchPanicLayer` 返回 500；`AppError` 不把 sqlx/Redis 原始错误返回客户端 |
| 错误响应统一 | ✅ | 全部 `{"error":"..."}`；内部错误记录日志，对外固定 `Internal server error` |
| 依赖漏洞 | ✅ | `cargo audit` exit 0（1 条 unmaintained 警告，见下）；`cargo deny check` advisories/bans/licenses/sources 全绿 |
| 依赖许可证 | ✅ | `deny.toml` 只允许 MIT/Apache/BSD/ISC/Unicode/Zlib/MPL/CC0/Unlicense/BSL；GPL/AGPL/未知许可证直接失败 |
| 依赖来源 | ✅ | 仅 crates.io；禁止 git / 未知 registry；禁止 wildcard 版本 |

## 依赖审计结果

`cargo audit`（2026-11-05，exit 0）：

```text
Warning: 1 allowed warning found
RUSTSEC-2026-0440  astral-tokio-tar 0.6.4  unmaintained
  └── testcontainers 0.27.3 (dev-dependency only)
```

`cargo deny check`：

```text
advisories ok, bans ok, licenses ok, sources ok
```

忽略项与理由（`.cargo/audit.toml`、`deny.toml`）：

| 公告 | 原因 |
|---|---|
| RUSTSEC-2023-0071（rsa Marvin） | 服务只做 HS256；sqlx 的 `mysql-rsa` 只用 RSA 公钥做 MySQL 密码交换，不涉及私钥运算 |
| RUSTSEC-2026-0387（crypto-bigint 时序） | jsonwebtoken 的 ECDSA 依赖，HS256 路径不可达 |
| RUSTSEC-2026-0440（astral-tokio-tar） | 仅 `testcontainers`（dev）使用，不进入生产二进制 |

Phase 7 同时移除了 `comrak` 的默认 feature（`cli` + `syntect-onig`），因此
`bincode`/`yaml-rust`/`onig` 不再进入依赖图；Markdown golden 25 例仍然全绿。

## 已知差异 / 后续项

1. **请求体上限**：Go 的 `ShouldBindJSON` 没有全局上限；Rust 对 JSON 路由使用
   axum 默认 2 MiB，超过返回 413。正常文章（Markdown 正文）远低于该值；如确实
   需要更大正文，调高 `DefaultBodyLimit` 或给编辑器加附件上传。
2. **上传魔数**：目前只校验扩展名和大小。后续可用 `infer` 校验 PNG/JPEG/GIF
   文件头；注意这会改变 Q8 的已批准差异（`.jpeg` 扩展名 + PNG 内容当前会通过）。
3. **可信代理**：`X-Forwarded-For` 完全信任。生产在反向代理后面时应增加
   `trusted_proxies` 配置，只接受来自代理的 XFF；本次保持 Go 行为。
4. **限流窗口**：`INCR` 后每次 `EXPIRE` 会重置 IP 窗口（Q12）。建议后续用 Lua
   实现固定窗口；切换期保持现状，避免行为变化。
5. **`user_id == 1` 短路**：保留现有长期 token 的兼容性；切换稳定后应评审移除，
   并同步更换 `SMILEYAN_BLOG_TOKEN`。
6. **JWT 存储**：前端把 token 放 localStorage（既有实现），存在 XSS 读取风险；
   CORS 为 `*` 是公开博客的既有契约。若未来引入敏感写操作，建议改为 HttpOnly
   Cookie + 明确 Origin 白名单。

## 复现

```bash
cargo audit
cargo deny check
cargo test --all-targets          # 含 JWT/上传/TTS/限流/契约测试
```

契约测试中的安全相关覆盖：

- `tests/auth_contract.rs`：缺 header、错误格式、过期 token、HS512、dev token、
  读者 token 访问 admin（403）。
- `tests/contract_full.rs`：全部 172 个 HTTP 夹具（含 401/403/429/400 文案）。
- `tests/tts_contract.rs`：认证、全部校验分支、上游错误透传、未配置 500。
- `tests/common/mod.rs`：`RecordingMailer` 断言邮件内容，不依赖真实 SMTP。
