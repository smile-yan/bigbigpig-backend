# Rust 与 Go 基线的 API 差异清单（Phase 7）

> 基线：tag `go-baseline-2026-11`（commit `1f3a87f`）
> Rust：本仓库 Phase 7 版本
> 验证方式：`tests/contract_full.rs` 按采集顺序重放全部 172 个 HTTP 夹具、
> 8 封邮件与 13 个 note；除下表的已批准差异外，状态码、响应 JSON、邮件正文、
> Redis 状态全部逐字段一致。

## 1. 已批准的差异（均为计划内修复）

| # | 计划项 | Go 行为 | Rust 行为 | 影响端点 / 夹具 |
|---|---|---|---|---|
| 1 | **Q1** 搜索删除 | 索引按 slug 建、按 id 删，删除后仍有残留，`total` 虚高 | 删除/更新/恢复统一按 slug 同步；无残留 | `search_after_soft_delete`、`search_after_restore`、`search_after_soft_delete_again`、`search_after_hard_delete` |
| 2 | **Q2** 搜索状态 | 启动时索引全部文章，草稿/隐藏/已删除可被搜到 | 只索引/返回 `published && !is_deleted && deleted_at IS NULL` | `search_draft_leak`、`search_hidden_leak`、`search_deleted_leak`、`search_published_is_deleted_leak`、`search_soft_deleted_at` |
| 3 | **Q2b** 公共文章泄漏 | 公共列表/详情不过滤 `is_deleted` | 公共列表/详情返回 404 或排除该行；分页随之前移；被泄漏详情的 `view_count` 不再自增 | `posts_default`、`posts_page2_size2`、`posts_category`、`posts_tag`、`posts_keyword`、`posts_invalid_page`、`posts_page_size_zero`、`posts_negative_page`、`post_detail_published_is_deleted_leak`，以及后续所有包含 post 9 的管理员列表 |
| 4 | **Q4** 页面草稿 | 公共 `GET /api/pages` 返回所有状态 | 公共列表只返回 published；公共路由使用可选认证，管理员 token 可见全部 | `pages_default`、`pages_parse_int_quirk`、`pages_page_size_zero`、`pages_after_delete` |
| 5 | **Q6** 摘要截断 | 按字节截断，可能产生 `U+FFFD` | 200 字节处回退到 UTF-8 边界 | 搜索结果中 `published-long` 的 `excerpt`（夹具中该字段以 `<q6-excerpt>` 归一化后比较） |
| 6 | **Q7** 默认昵称 | `email[:len-10]`，短邮箱 panic | 取 `@` 前缀；短邮箱正常注册 | `login_short_email_panic`（500 → 200）、`login_new_user`/`user_get_new_user`（昵称 `newuser@e` → `newuser`）、`admin_users`（多出 `a@b.co`，后续用户 id 顺延为 101） |
| 7 | **Q8** 头像扩展名 | 取最后 4 字符，`.jpeg` 永远被拒；大小限制未生效 | `.jpg/.jpeg/.png/.gif` 大小写不敏感；`upload.max_size` 生效 | `upload_avatar_jpeg_ext`（400 → 200） |
| 8 | **Q10** 回复邮件链接 | 相对路径 `/post/...#comment-...` | 绝对 URL `https://smileyan.cn/post/...#comment-...` | `email_comment_reply_notification` |
| 9 | **Q11** TTS 长度 | `len(input)` 字节，中文约 1365 字 | 按字符数 4096 校验 | `tts_input_cjk_byte_quirk`（1366 汉字 400 → 200） |
| 10 | **Q20** JWT 算法 | 接受 HS256/384/512 | 只接受 HS256 | `admin_route_hs512_token`（200 → 401） |
| 11 | **Q9** 通知并发 | 串行发信，错误被忽略 | `buffer_unordered(5)` 并发，记录成功/失败数；响应 `{message,count}` 不变 | `admin_notify`（响应形状不变；邮件顺序可能不同） |
| 12 | **Q16** 上传目录 | `createUploadDirs()` 空实现 | 启动时创建 `uploads/`、`uploads/avatars/` | 首次上传不再依赖目录预建 |

> 差异 3 的二阶影响：Go 的泄漏详情 `GET /api/posts/published-but-is-deleted`
> 会把 post 9 的 `view_count` 从 8 加到 9；Rust 返回 404，因此后续夹具中该行
> 的 `view_count` 始终是 8。重放测试对 post 9 的 9 做了归一化。

## 2. 保留的 Go 怪癖（不属于差异）

| 计划项 | 保留的行为 |
|---|---|
| Q3 | 公共文章路由没有认证，`?edit=true` 只关闭阅读计数，不能预览草稿 |
| Q5 | 软删除把 slug 改成 8 位 hex；恢复不还原原 slug |
| Q12 | IP 限流每次 `INCR` 后都 `EXPIRE`，窗口会被重置 |
| Q13 | 页面/订阅分页用自实现 `parseInt`（`"10abc" → 10`）；文章/评论用 `Atoi`（失败为 0） |
| Q14 | 未 Preload 的 `category`/`author`/`post` 输出零值对象；`tags` 可能是 `null` 或 `[]` |
| Q17 | 分类/标签软删除后文章仍引用，Preload 得到零值对象 |
| Q18 | 评论只返回一层 `children`；Q18b：回复同时作为顶层项返回 |
| Q19 | 软删除的订阅邮箱再次订阅 → 500 `Failed to subscribe` |
| Q21 | 方法不匹配返回 Gin 的 404（不是 axum 的 405），且无 `Allow` 头 |
| Q22 | 尾斜杠 301（GET）/307（其他方法），带 `Location` |
| Q23 | panic 返回 500；`CatchPanicLayer` 保证连接不中断 |
| Q24 | 时间输出 Go RFC3339Nano 风格（`+08:00` 偏移、DATETIME(3) 0–3 位小数） |
| Q25 | `view_count` 自增同时刷新 `updated_at` |
| Q26 | 搜索解析失败返回 500 `Search failed`（未改成 400） |
| Q27 | 合成 id=1 管理员 `PUT /api/user` → 500；头像上传写入文件但不更新 `users.avatar` |
| — | `tag_ids: []` 在更新时仍会加载全部标签（GORM 关联替换怪癖） |

## 3. 新增的行为差异（安全/实现约束，非计划修复）

| 差异 | 说明 | 风险 |
|---|---|---|
| JSON 请求体上限 2 MiB | axum 默认 `DefaultBodyLimit`；超限返回 413。Go 无上限 | 极长文章（>2 MiB Markdown）会被拒；正常文章远低于此值 |
| 搜索相关性排序 | tantivy 与 Bleve 打分不同：命中集合与 `total` 一致，但同一页内顺序、`page_size=1` 的分页内容可能不同 | 前端按相关性展示，用户可见；Phase 5 语料 top-10 命中 100% 重合 |
| 搜索 `highlights` 字段 | 仍不返回（与 Go 一致）；前端 `highlights?.[0]` 继续走 fallback | 无新增差异 |
| TTS 上游请求头 | 不带 `Accept-Encoding: gzip`，User-Agent 不同 | 上游只依赖 `Content-Type`/`Authorization`/JSON body，API 响应不变 |
| 日志格式 | JSON 文件日志 + 控制台；请求日志字段对齐 Go | 巡检脚本若依赖 Go 文本格式需调整 |
| 新增只读端点 `GET /api/posts/dates` | 首页日历改用日期聚合（方案 A，2026-11-10 新增），Go 基线无此端点；响应为裸数组 `[{date,count}]`，只统计 published/未删除文章 | 增量接口；旧前端不调用，回滚只需前端回退 |
| 新增只读端点 `GET /api/posts/summary` | 首页列表改用轻量卡片端点（2026-11-10 新增），Go 基线无此端点；参数与 `GET /api/posts` 相同，但不返回 `content`/`html_content`/`author`，分类随主查询 JOIN，`category` 缺失时为 `null`、`tags` 恒为 `[]`；`excerpt` 为空时仍从正文前缀生成 | 增量接口；`AdminSubscriptions.vue` 仍用 `/api/posts`，回滚只需前端改回原路径 |

## 4. 完全一致的契约

以下内容在重放测试中逐字段一致，无需前端/调用方改动：

- 43 个 Go 基线端点（含 `/api/hello`、`/uploads/*`、`/v1/audio/speech`）的路径、方法与认证层级；
  新增的 `GET /api/posts/dates`、`GET /api/posts/summary` 见第 3 节。
- 成功响应形状：列表 `{list,total,page[,page_size]}`、分类/标签/评论裸数组、详情裸对象；
  **没有** `{data:...}` 包装。
- 错误体 `{"error":"..."}` 与固定文案（401/403/404/400/429/500）。
- JWT：HS256、claims 字段、`SMILEYAN_BACKEND_JWT_SECRET` 互通；Go 与 Rust 双向可用。
- 数据库表/列/索引不变；软删除、`Unscoped`、`post_tags`、`AUTO_INCREMENT` 行为一致。
- Redis key 与 TTL：`verification_code:*`、`rate_limit:email:*`、`rate_limit:ip:*`。
- 环境变量名、`config.yaml` 字段（新增 `site_url`/`search`/`tts`，均有默认值）。
- CORS、尾斜杠、404 文案、Content-Type（`application/json; charset=utf-8`）。
- Markdown `HTMLContent`：25 个 golden 用例逐字节一致（仅两类已记录的语义等价差异：
  智能标点实体编码、goldmark `'twas` 启发式）。
- 邮件：验证码、订阅确认、回复通知（除 Q10 绝对链接）、新文章通知的标题/正文。

## 5. 验证命令

```bash
cargo test --test contract_full      # 172 HTTP + 8 emails + 13 notes
cargo test --all-targets             # 单元 + 各阶段契约 + Markdown golden
./scripts/bench.sh                   # Go vs Rust 性能（见 docs/performance.md）
cargo audit && cargo deny check      # 依赖安全（见 docs/security-checklist.md）
```
