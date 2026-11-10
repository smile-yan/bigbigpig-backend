# 兼容性决策表（Q1–Q24）

> Phase 0 交付物 · 2026-11-04
> 基线：`smileyan-backend` 分支 `phase0/go-baseline`，commit `1f3a87f`，tag `go-baseline-2026-11`
> 目标：逐项决定“修复 / 保留 / 待定”，作为 Rust 实现与契约测试的验收依据。

## 决策口径

- **修复**：Rust 版改变行为（修复安全/崩溃/数据泄漏）。必须在契约测试中标注“已批准差异”。
- **保留**：Rust 版逐字节/逐状态码复刻，即使看起来是怪癖。契约夹具锁定。
- **待定**：Phase 0 不拍板，切换前评审；Rust 默认按“保留”实现。

状态码、错误文案、JSON 形状、Redis key、HTML class 属于**冻结契约**，不在“顺手优化”范围。

---

## 1. 缺陷决策

| ID | 问题 | 证据 / 位置 | 决策 | Rust 实施要点 | Phase |
|---|---|---|---|---|---|
| Q1 | 搜索索引以 `slug` 为 key，删除却按 `id` 删除 | `services/search.go:83`（`Index(post.Slug)`）、`:95`（`Delete(fmt.Sprint(id))`） | **修复** | 索引文档存 `slug` 字段；删除/更新按旧 slug `delete_term`，再按新 slug 写入。契约测试覆盖“软删→恢复→硬删”后的 `total`。 | 5 |
| Q2 | 启动索引包含 draft/hidden/`is_deleted` 文章，搜索查询不过滤状态 | `main.go:47`（`Find(&posts)` 只受 `deleted_at` 约束）、`services/search.go:134` | **修复** | 只索引 `status='published' AND is_deleted=0 AND deleted_at IS NULL`；查询结果同样过滤（防止索引陈旧）。 | 5 |
| Q2b | **新发现**：公共文章列表/详情不过滤 `is_deleted` | `controllers/post.go:79-88`（列表只加 `status='published'`）、`:135-148`（详情只判断 status） | **修复** | 公共 `GET /api/posts`、`GET /api/posts/:slug` 增加 `is_deleted=false`。种子 `published-but-is-deleted` 锁定该泄漏。 | 3 |
| Q3 | 公共 `GET /api/posts/:slug?edit=true` 的管理员判断永远失效 | `routes/routes.go:26`（公共路由无 Auth）、`controllers/post.go:142-145` | **保留** | 公共 posts 路由不加可选认证；`edit=true` 对匿名/管理员都不改变可见性。前端从未调用 `edit=true`。 | 3 |
| Q4 | 公共 `GET /api/pages` 不按 status 过滤；页面详情也无法给管理员预览草稿 | `controllers/page.go:14-30`、`:54-60` | **修复** | 给公共 pages 路由加**可选认证**：匿名/reader 只返回 `published`，管理员可见全部；`GET /api/pages/:slug` 同样。管理员前端（`AdminPages.vue`）继续工作。 | 3 |
| Q5 | 软删文章把 slug 改成 8 位随机码，恢复不还原原 slug | `controllers/post.go:349`、`:376-378` | **保留** | 恢复后 slug 仍为随机码；不新增 `original_slug` 列。恢复后 status=draft，**再删一次仍是软删**，只有对 hidden 状态再删才硬删；契约测试覆盖三次 DELETE。 | 3 |
| Q6 | 摘要按字节下标截断 | `controllers/post.go:557-559`、`services/search.go:141-145` | **修复** | 按 `char`/rune 边界截断。**更正计划描述**：Go 不会 panic，但会产生非法 UTF-8，`encoding/json` 输出 `\ufffd`；Rust 必须避免替换字符。 | 3/5 |
| Q7 | 登录默认昵称 `email[:len-10]`，短邮箱 panic | `controllers/user.go:99` | **修复** | 取 `@` 前部分并安全截断；`a@b.co` 这类短邮箱不再 500。契约夹具记录 Go 的 500 panic。 | 2 |
| Q8 | 头像扩展名取最后 4 字符；`max_size` 未生效；上传目录未创建 | `controllers/user.go:198-199`、`main.go:89` | **修复** | `Path::extension()` + 白名单（大小写不敏感，含 `.jpeg`）；按 `upload.max_size` 限制；启动 `create_dir_all`。文件名去路径、过滤控制字符。 | 2 |
| Q9 | 通知订阅者串行发信，错误被忽略 | `controllers/subscription.go:143-154` | **修复** | `buffer_unordered(5)` 并发；统计成功/失败；响应仍是 `{"message":"Notifications sent","count":<活跃订阅数>}`（count 语义不变）。 | 4 |
| Q10 | 回复评论邮件链接是相对路径 | `controllers/comment.go:114` | **修复** | 新增 `site_url`（默认 `https://smileyan.cn`）；链接改绝对 URL；HTML 转义用户输入。 | 4 |
| Q11 | TTS `input` 用字节长度；上游请求无超时 | `controllers/tts.go:53`、`:146-147` | **修复** | 按字符数 4096 校验；`connect_timeout=10s`，非流式总超时 120s，SSE 不设总超时。注意这会放宽中文输入（Go 约 1365 字）。 | 6 |
| Q12 | 限流 `INCR` 后每次都 `EXPIRE` | `utils/email.go:119-120` | **保留** | 保持固定窗口重置行为；后续改 Lua 原子窗口单独评审。 | 2 |
| Q13 | 自实现 `parseInt` 忽略非数字字符 | `controllers/page.go:153-161`、`controllers/subscription.go:99-100` | **保留** | Rust helper `parse_digits_prefix()` 复刻：`"1abc"→1`、`"2xyz"→2`。`page_size=0` 会生成 `LIMIT 0` 返回**空 list**（total 不变）；`page=-1` 的负 offset 被忽略，仍返回数据。契约测试覆盖。 | 3/4 |
| Q14 | GORM 对结构体字段的 `omitempty` 无效 | `models/models.go:99-102`、`:146-151` | **保留** | 未 Preload 时仍输出零值 `category`/`author`/`post` 对象；`tags` 可能是 `null`。契约夹具锁定每个端点的形状。 | 3 |
| Q15 | 启动时 `AutoMigrate` | `models/migrate.go:7-31` | **修复** | Rust 改用 `sqlx::migrate!` 显式 migration；baseline 用 `CREATE TABLE IF NOT EXISTS`。Go 在切换后不再运行。 | 1 |
| Q16 | `createUploadDirs()` 是空实现 | `main.go:89-92` | **修复** | Rust 启动时 `create_dir_all(upload.path)`、`create_dir_all(upload.avatar_path)`。采集脚本已手动创建目录以复刻生产。 | 1/6 |
| Q17 | 分类/标签软删除后文章仍引用 | `controllers/post.go:478-485`、`:538-545` | **保留** | Preload 过滤 `deleted_at`；文章的分类/标签可能变成零值或缺失。契约夹具覆盖。**注意**：GORM AutoMigrate 可能给 `post_tags` 加外键，`DeletePost` 硬删错误被忽略；采集脚本先清空标签再硬删，Rust 需确认外键/级联策略。 | 3 |
| Q18 | 评论只 Preload 一层 `Children.User` | `controllers/comment.go:19` | **保留** | 只挂一层 children；更深层级不返回。 | 4 |
| Q18b | **新发现**：公共评论列表不筛选 `parent_id IS NULL`，回复会同时作为顶层项返回 | `controllers/comment.go:19-36` | **保留（待夹具确认）** | Rust 必须复刻“所有 approved 评论都是顶层，且各自带一层 children”的行为；前端可能靠 `parent_id` 自行过滤。采集脚本已放入二级回复（comment 8）验证。 | 4 |
| Q19 | 订阅软删后同邮箱无法重新创建 | `controllers/subscription.go:24-47` | **保留** | Go 查询排除软删行 → 插入撞唯一索引 → 500 `Failed to subscribe`。Rust 保持，不改成 upsert。契约夹具覆盖 `deleted-sub@example.com`。 | 4 |
| Q20 | Go 接受 HS256/384/512 | `middleware/auth.go:47-49`（keyfunc 不校验 alg） | **修复（更严格）** | Rust 只接受 HS256；现有 token 均为 HS256。契约夹具记录 Go 接受 HS512。 | 2 |
| Q21 | Gin 方法不匹配返回 404；axum 默认 405 | — | **保留** | `Router::method_not_allowed_fallback` 返回 404 + `404 page not found`。 | 1 |
| Q22 | Gin 默认 301 重定向尾斜杠 | — | **保留** | `NormalizePathLayer::trim_trailing_slash()`。契约夹具记录 `/api/posts/` 的 301 + Location。 | 1 |
| Q23 | Gin `Default()` 含 Recovery；panic 返回 500 | `main.go:64` | **修复** | `CatchPanicLayer`；panic 返回 500，不断连接。契约夹具记录短邮箱登录的 500。 | 1/2 |
| Q24 | 时间 `loc=Local`，Go 输出 RFC3339Nano + 本地偏移 | `config/config.go:170` | **保留** | 用 `NaiveDateTime` 读 `DATETIME(3)`，按配置时区（默认 `Asia/Shanghai`）序列化。**更正计划**：不要固定 `SecondsFormat::Millis`；Go 会省略全零小数并裁剪尾随零，`DATETIME(3)` 实际只会出现“无小数”或“1–3 位小数”。 | 3 |
| Q25 | **新发现**：浏览文章自增 `view_count` 会顺带更新 `updated_at` | `controllers/post.go:152`（GORM `Update` 自动维护 `updated_at`） | **保留** | Rust 的 `UPDATE posts SET view_count=view_count+1, updated_at=NOW(3)`；否则详情响应里的 `updated_at` 与 Go 不一致。 | 3 |
| Q26 | **新发现**：搜索 QueryString 语法错误返回 500 | `controllers/search.go:22-25` | **保留** | 查询 `"` → 500 `{"error":"Search failed"}`；Rust 保持 500/文案，不改为 400。 | 5 |
| Q27 | **新发现**：`user_id == 1` 的合成管理员 `DB.Save` 必然失败（零值 `created_at`） | `middleware/auth.go:121-129`、`controllers/user.go:169`、`:219` | **保留** | `PUT /api/user` 对 id=1 返回 500 `Failed to update user`，且不改数据库行；`upload-avatar` 仍写文件并返回 200，但跳过 `users.avatar` 更新。Phase 2 集成测试锁定。 | 2 |

---

## 1b. 实测佐证（2026-11-04，tag `go-baseline-2026-11`）

夹具位于 `tests/fixtures/contract/`，以下均可在对应文件复核。

| ID | 夹具 / 观测结果 |
|---|---|
| Q1 | `search_after_soft_delete` total=4/list=3；`search_after_restore` total=5/list=4；`search_after_soft_delete_again` total=5/list=3；`search_after_hard_delete` total=5/list=3。索引残留旧 slug，`total` 大于可回表条数。 |
| Q2 | `search_draft_leak` 返回 `draft-post`；`search_hidden_leak` 返回 `hidden-post`；`search_deleted_leak` 返回 `deletedpost`；`search_published_is_deleted_leak` 返回 `published-but-is-deleted`。 |
| Q2b | `posts_default` 与 `post_detail_published_is_deleted_leak` 都返回 id=9、`is_deleted=true` 的文章。 |
| Q3 | `post_detail_draft_edit_admin_token`（管理员 token + `edit=true`）→ 404 `Post not found`。 |
| Q4 | `pages_default` 返回 `about` + `draft-page`；`page_detail_draft_public` → 404。 |
| Q5 | `admin_get_restored_post`：status=draft、slug=`<random-slug>`；`admin_delete_post_soft_again` 仍返回 `Post deleted`；第三次 `admin_delete_post_hard` 才成功，随后 `admin_get_hard_deleted_post` → 404。 |
| Q6 | `posts_default` 的 `getExcerpt` 在本语料恰好切在字符边界；`zh_01` 搜索结果的长文摘要末尾出现 `U+FFFD`（`...测�...`），确认非法 UTF-8。 |
| Q7 | `login_short_email_panic`（`a@b.co`）→ 500 且 body 为空；`admin_users` 中新用户昵称 `newuser@e`。 |
| Q8 | `upload_avatar_jpeg_ext` → 400（`.jpeg` 被拒）；`upload_avatar_ok` → 200 + `/uploads/avatars/2_capture-avatar.png`；`get_uploaded_avatar` → 200。 |
| Q9 | `admin_notify` 只有 1 个活跃订阅者（count=1），未做并发/失败统计压力验证；邮件 `email_notify_subscriber` 已捕获。 |
| Q10 | `email_comment_reply_notification` 的链接为 `/post/published-post#comment-101`（相对路径）。 |
| Q11 | `tts_input_cjk_byte_quirk`（1366 个汉字）→ 400；`tts_binary`（1365 个汉字）→ 200。 |
| Q12 | Redis notes：首次 send-code 后 email key 存在、IP count=1；重复请求 429 后 TTL/count 不变；管理员邮箱无 `rate_limit:email` key。 |
| Q13 | `pages_parse_int_quirk`（`page=1abc&page_size=2xyz`）→ 返回 2 条；`posts_page_size_zero` → `list=[] total=5`；`posts_negative_page` → `page=-1` 但仍有数据。 |
| Q14 | 无分类文章的 `category` 为零值对象；评论响应中的 `post`/`user` 也是零值完整对象；`tags` 可能为 `null`。 |
| Q17 | 硬删前先清空 `post_tags`（`admin_update_post_clear_tags`）后硬删成功。GORM 是否真的创建 `post_tags` 外键、以及不清标签时硬删是否失败，未单独验证。 |
| Q18b | `comments_by_post` 顶层 ids = [8,3,2,1]；id=8（parent=3）同时是 id=3 的 child，id=3（parent=2）同时是 id=2 的 child——回复会重复出现在顶层与 children。 |
| Q19 | `subscribe_soft_deleted_duplicate`（`deleted-sub@example.com`）→ 500 `Failed to subscribe`。 |
| Q20 | `admin_route_hs512_token` → 200（Go 接受 HS512）。 |
| Q21 | `method_not_allowed`（DELETE `/api/hello`）→ 404 `text/plain` `404 page not found`。 |
| Q22 | `trailing_slash`（GET `/api/posts/`）→ 301，`Location: /api/posts`，body 为 Gin 的 HTML 跳转页。 |
| Q23 | `login_short_email_panic` → 500（Gin Recovery），连接未断开。 |
| Q24 | `time_format_sample_seeded.created_at` = `2026-10-01T10:00:00+08:00`（无小数）；`time_format_sample_created` 观测到 `.6` / `.604`，即 1–3 位裁剪后小数。 |
| Q25 | 读取 `post_detail_published` 后，同一文章的 `updated_at` 变成当前时间（`time_format_sample_seeded`）。 |
| Q26 | 搜索 `special_01`（keyword=`"`）→ 500 `{"error":"Search failed"}`。 |
| Q27 | 用 Go 基线 + fixture MySQL 实测：管理员 token `PUT /api/user` → 500 `Failed to update user`，row 1 未变化；`POST /api/upload-avatar` → 200 + `/uploads/avatars/1_admin-avatar.png`，但 `users.avatar` 仍为空，随后 `GET /api/user` 仍返回合成对象。 |

---

## 2. 新发现的契约风险（来自前端核查）

这些不在 Q1–Q24 中，但会直接导致 Rust 版前端回归。

| ID | 风险 | 证据 | 处理 |
|---|---|---|---|
| C1 | `GET /api/pages` 同一响应被前端按两种形状使用：`App.vue` 当数组，`AdminPages.vue` 当 `{list,total}` | `smileyan-frontend/src/App.vue:139`、`AdminPages.vue:92`；Go 始终返回 `{list,total}` | **冻结 Go 形状**。不要为了“修前端”改成数组；如需修前端，另开前端任务。 |
| C2 | 所有成功响应**没有** `{data:...}` 包装 | 30+ 调用点均直接读 `res.data` | 保持裸对象/数组。 |
| C3 | `list` 被无空值保护地解引用（`.forEach`/`.filter`） | `Home.vue:193,212`、`AdminPosts.vue:164`、`AdminComments.vue:72`、`AdminPages.vue:92`、`AdminSubscriptions.vue:77` | Rust 列表必须输出 `[]` 而不是 `null`（尤其无结果时；Go 部分端点会输出 `null`，见夹具）。 |
| C4 | `GET /api/categories`、`/api/tags`、`/api/comments` 返回**裸数组** | `Home.vue:180,185`、`Post.vue:343` | 不要统一成 `{list,total}`。 |
| C5 | `comment.user` 必须始终是对象 | `Post.vue:78-79,92-93` 无 optional chaining | Preload 缺失/用户软删时也要输出零值 `User` 对象（Go 行为）。 |
| C6 | `post.category` 在未 Preload/分类软删时是零值对象，不是 `null` | `models.go:99`；`Home.vue:35`、`Post.vue:35` | 保持零值对象以逐字节对齐；前端 `v-if` 会渲染空标签（现有行为）。 |
| C7 | 搜索响应没有 `highlights`，前端仅作兜底 | `Home.vue:38` | v1 不新增字段；保留 `excerpt`。 |
| C8 | 分类/标签列表必须带 `post_count` | `SidebarContent.vue:31,47` | 按 `published AND is_deleted=0` 统计。 |
| C9 | 日期必须是 `new Date()`/dayjs 可解析的字符串 | `Home.vue:193,280`、`Post.vue:372` 等 | 严格复刻 RFC3339 本地偏移，不要输出 epoch 数字。 |
| C10 | 枚举必须小写字符串 | `store/user.js:18`、`AdminPosts.vue:232-240`、`AdminComments.vue:106-114` | `admin/reader`、`published/draft/hidden`、`pending/approved/rejected`。 |
| C11 | 创建文章读取 `res.data.id ?? res.data._id` | `Editor.vue:827` | 输出 `id`。 |
| C12 | 管理员调用公共 `GET /api/posts` **不会**返回草稿 | `routes.go:26` 无 Auth；`post.go:82-88` | 前端 `AdminSubscriptions.vue` 只拿到 published；不要“顺手”加可选认证改变它。 |
| C13 | 错误体统一 `{"error":"..."}` | `App.vue:162,182,210`、`Post.vue:362`、`Editor.vue:839` | 所有非 2xx 保持该形状。 |
| C14 | approve/reject 发送 `{}` 空 JSON body | `AdminComments.vue:78,88` | Rust handler 必须接受空对象，不能要求非空 body。 |
| C15 | `/uploads/...` 由后端直接提供，前端不拼 URL | `routes.go:11` | Rust 保留 `ServeDir`；头像/封面 URL 原样返回。 |

---

## 3. 计划中需要更正的描述

| 计划原文 | 实际情况 |
|---|---|
| Q6“中文长文可能 panic” | 不会 panic；字节切片产生非法 UTF-8，JSON 编码替换为 `\ufffd`。仍按 P0 修。 |
| Q2“搜索查询不再过滤状态” | 准确；另需补充 Q2b：公共 posts 列表/详情本身也不过滤 `is_deleted`。 |
| Q4“管理员需要单独接口或 `?all=true`” | 推荐**可选认证**：管理员 token 可见全部，匿名仅 published；`AdminPages.vue` 无需改动。 |
| Q24“输出 `SecondsFormat::Millis`” | Go 会省略全零小数；应实现“nanos==0 → 无小数，否则 3 位（DATETIME(3)）”的 `go_time`。 |
| §4.5 关联对象“Rust 侧不要简单用 skip_serializing_if” | 同意；用夹具逐端点锁定。`comment.user` 必须非空对象，`post.category` 保持零值对象。 |
| §1.5 Q3 描述“草稿预览接口实际不可用” | 准确；前端未使用 `edit=true`，保留即可。 |
| §4.3 管理员用户角色路由 `:id` | 准确，保持 `:id` 而非 `_id`。 |

---

## 4. 待确认决策（§11）当前状态

| # | 问题 | 当前决定 |
|---|---|---|
| 1 | 仓库形态 | **已定**：同级独立仓库 `smileyan-backend-rs`（用户 2026-11-04 决定） |
| 2 | 怪癖保留策略 | 按上表：Q1/Q2/Q2b/Q4/Q6/Q7/Q8/Q9/Q10/Q11/Q15/Q16/Q20/Q23 修复，其余保留 |
| 3 | 搜索分词 | 先 jieba + LIKE 回退；用 `tests/fixtures/search` 语料验证后再决定单字双字段 |
| 4 | 时间格式 | 严格复刻 Go 本地偏移 RFC3339 |
| 5 | TTS 是否纳入 v1 | 纳入；Go 基线已提交 TTS |
| 6 | 切换方式 | 首次维护窗口直接切；灰度后续评估 |
| 7 | `user_id == 1` 短路 | **保留**（兼容现有长期 token）；Phase 7 安全评审 |
| 8 | 是否新增 `highlights` | v1 不新增，保持契约快照 |

---

## 5. Phase 0 采集状态

| 项目 | 状态 | 说明 |
|---|---|---|
| Go 基线提交 + tag | ✅ 完成 | `1f3a87f` / `go-baseline-2026-11` |
| schema 基线 | ✅ 完成 | `tests/fixtures/db/0001_schema.sql`（按 `docs/database_schema.sql` + GORM 索引） |
| 脱敏 seed | ✅ 完成 | `tests/fixtures/db/seed.sql`，固定 id/时间，覆盖 published/draft/hidden/is_deleted/deleted_at |
| Markdown golden | ✅ 已执行 | 25 个用例；`scripts/capture_markdown.sh --check` 通过 |
| HTTP 契约 | ✅ 已执行 | 172 个 HTTP 用例 + 8 封邮件 + 13 条 note；连续两次采集 `diff -r` 干净 |
| 搜索语料 | ✅ 已执行 | 54 条：52×200、1×400（`keyword=`）、1×500（`"`）；连续两次采集一致 |
| TTS/邮件采集 | ✅ 已执行 | TTS 二进制/SSE/错误透传、6 种邮件模板均已捕获 |
| 前端实际调用核查 | ✅ 完成 | 见 `docs/frontend-api-usage.md` |
| 决策表 | ✅ 完成 | 本文 |

采集环境：Docker 29.5.3（MySQL 8.4 / Redis 7.4 / MailHog），Go 基线通过
`git worktree` 检出在 tag `go-baseline-2026-11` 上执行，`index.json.meta`
记录 `backend_commit=1f3a87f` 与 `backend_tag=go-baseline-2026-11`。

> Phase 0 的验收条件（可在干净环境重复生成相同夹具）**已达成**。
> 后续若 Go 基线变化，重新运行两个采集脚本并逐字节审查 diff；夹具变更需
> 与契约测试的“已批准差异”清单同步。

---

## 6. Phase 2 实施记录（2026-11-04）

实现分支：`smileyan-backend-rs` 的 `phase2/auth-users`。

### 6.1 已实现端点

| 方法 | 路径 | 行为 |
|---|---|---|
| POST | `/api/send-code` | 邮箱校验、管理员绕过、Redis 限流、600 s 验证码、SMTP 验证邮件 |
| POST | `/api/login` | 验证码校验/消费、按邮箱查用户或懒创建、签发 HS256 JWT |
| GET | `/api/user` | claims → 用户；`user_id=1` 合成管理员；0/不存在 → 401 `User not found` |
| PUT | `/api/user` | 更新 nickname/avatar；Q27 id=1 保留 500 |
| POST | `/api/upload-avatar` | `.jpg/.jpeg/.png/.gif`（大小写不敏感）、`max_size`、文件名净化、ServeDir |
| GET | `/api/admin/users` | `{list,total,page}`，Go 时间格式，Atoi/GORM 分页语义 |
| PUT | `/api/admin/users/:id/role` | 任意 role 字符串、404 `User not found`、写失败仍 200 |

### 6.2 验证结果

| 项 | 结果 |
|---|---|
| JWT Go→Rust | 用 Go `golang-jwt/jwt/v5` 签发的固定 HS256 token（`tests/auth_contract.rs::GO_SIGNED_READER_TOKEN`）通过 `/api/user` 验证 |
| JWT Rust→Go | `cargo run --bin genjwt` 生成的 dev/admin token 用 Go 基线同一库解析成功（`valid=true alg=HS256`，claims 一致） |
| 验证码/限流 | Redis keys、TTL（600/600/3600）、管理员绕过、重复 429、IP 计数阈值 100 均有集成断言 |
| 邮件 | `RecordingMailer` 断言收件人、标题 `Smileyan 验证码：<code>`、HTML 正文逐字节一致；另测 SMTP 失败 → 500 `Failed to send email` 且验证码/限流已写入 |
| MailHog E2E | release 二进制 + 真实 MailHog（`use_ssl=false`）：`send-code` 200 → 收到的标题/正文/验证码与 Go 一致 → `login` 200（新用户 id=100、昵称 `e2e`）→ `/api/user` 200 |
| 头像 | 扩展名/大小/路径穿越/ServeDir 200 与字节一致；Q8 差异明确 |
| 用户 | 新用户 id=100、昵称 local part；短邮箱 `a@b.co` 200；软删用户重复创建 → 500 `Failed to create user` |
| Q27 | 用 Go 基线 + fixture MySQL 实测锁定；Rust 集成测试复刻 |
| 质量门禁 | 35 单元 + 2 集成测试；`cargo fmt --check`、`clippy -D warnings`、`cargo build --locked --release`、`cargo audit`（仅既有 2 个 allowed warning）全部通过 |

### 6.3 相对计划的实现偏差（已批准）

| 项 | 偏差 | 原因 |
|---|---|---|
| Q7 | 新用户昵称取 `@` 前部分（`newuser`），短邮箱登录返回 200 | 计划明确修复；Go 会 panic（500） |
| Q8 | `.jpeg` 与大写扩展名接受；超过 `upload.max_size` → 400 `File too large`；启动/首次上传建目录 | 计划明确修复；Go 用最后 4 字符且不限制大小 |
| Q20 | HS512 token → 401 `Invalid or expired token` | 计划明确更严格；Go 接受 HS384/512 |
| Q23 | 短邮箱登录不再 panic | 与 Q7 相同 |
| Q27 | id=1 合成管理员的写路径保留 Go 的 500/不落库 | Phase 2 实测发现，避免“顺手修复”改变契约 |
| 时区 | 新增可选 `server.timezone`（默认 `Asia/Shanghai`），Rust 用它序列化 `DATETIME` | 计划 §5.6 建议；避免部署机 TZ 影响 `created_at` 字符串 |
| SMTP TLS | `use_ssl=false` 用 `Tls::Opportunistic`（有 STARTTLS 则升级，否则明文），而非强制 `starttls_relay` | 与 go-mail 默认 `OpportunisticStartTLS` 一致，MailHog 可用 |
| 限流顺序 | 先 `SetRateLimit`，再生成/保存验证码、发邮件 | 与 Go 代码一致；修正计划 §6.2 第 6 步“成功后写限流”的表述 |
| 空列表 | `GET /api/admin/users` 空结果输出 `[]`（Go 可能 `null`） | 前端未调用该端点；遵循 C3“列表不返回 null”的更安全行为 |
| 未知 admin 路径 | `/api/admin/*` 未实现路径在未认证时返回 401/403（Gin 对未注册路径直接 404） | Phase 2 需要让 `/api/admin/posts` 等已存在路径先过鉴权；后续阶段注册全部路由后由 Phase 7 契约测试复核 |

### 6.4 配置变更

- 新增 `server.timezone`（可选，默认 `Asia/Shanghai`），`config.yaml` 已写入。
- `SMILEYAN_BACKEND_DEV_TOKEN` 在启动时读入 `AppConfig`（不是 YAML 字段）；语义与 Go 的每请求 `os.Getenv` 一致。
- 未新增/重命名任何 `SMILEYAN_BACKEND_*` 变量，未改变 Redis key、表结构或 JWT secret。

---

## 7. Phase 3 实施记录（2026-11-05）

实现分支：`smileyan-backend-rs` 的 `phase3/content-domain`。

### 7.1 已实现端点

| 方法 | 路径 | 行为 |
|---|---|---|
| GET | `/api/posts` | 公共列表；published + 未 `is_deleted` + `deleted_at IS NULL`；分类/标签/关键字/日期筛选；Atoi 分页；空摘要按 Q6 截断 |
| GET | `/api/posts/:slug` | 公共详情；Q2b/Q3 可见性；非 `edit=true` 时 `view_count+1` 并刷新 `updated_at`（Q25） |
| GET | `/api/categories` | 裸数组，`LEFT JOIN` 一次算出 `post_count` |
| GET | `/api/tags` | 裸数组，`COUNT(DISTINCT p.id)` 的 `post_count` |
| GET | `/api/pages` | Q4 可选认证：匿名/reader 仅 published，admin 全部；`{list,total}` |
| GET | `/api/pages/:slug` | 同上；管理员可预览草稿 |
| GET | `/api/admin/posts` | `{list,page,page_size,total}`；status/category/tag/keyword 筛选；不加载 Author |
| GET | `/api/admin/posts/slug/:slug` | 管理员按 slug 详情（含 Category/Tags/Author） |
| GET | `/api/admin/posts/_id/:id` | 管理员按 id 详情；`deleted_at` 行 404 |
| POST | `/api/admin/posts` | slug 生成/Unscoped 唯一性检查、Markdown 渲染、标签关联、201 |
| PUT | `/api/admin/posts/_id/:id` | 空字符串=不修改；标签 `Replace`；Save 错误忽略后仍 200 |
| DELETE | `/api/admin/posts/_id/:id` | hidden → 硬删；其他 → `is_deleted=1,status=hidden` + 8 位随机 slug |
| POST | `/api/admin/posts/_id/:id/restore` | `is_deleted=0,status=draft`，slug 不恢复 |
| POST/PUT/DELETE | `/api/admin/categories...` | 创建/更新/软删；`sort>0` 才更新；Save 错误忽略 |
| POST/DELETE | `/api/admin/tags...` | 创建/软删 |
| GET/POST/PUT/DELETE | `/api/admin/pages...` | 原始字符串 id；创建默认 published；软删 |

### 7.2 决策执行

| ID | 结果 |
|---|---|
| Q1/Q2 | 搜索索引仍留到 Phase 5；Phase 3 的写路径未调用索引（与 Phase 5 一并接入） |
| Q2b | **已修复**：公共 posts 列表/详情排除 `is_deleted`。`posts_default` 由 5 条降为 4 条；`published-but-is-deleted` 由 200 改为 404 |
| Q3 | **保留**：公共 posts 路由不挂任何认证，`edit=true` 仅抑制计数 |
| Q4 | **已修复**：公共 pages 挂 optional auth；匿名/reader 只看到 published，admin 看到全部；`GET /api/pages` 匿名 `total` 由 2 改为 1 |
| Q5 | **保留**：软删生成 8 位 hex slug；恢复保持随机 slug、status=draft；再次 DELETE 仍软删，hidden 再删才硬删 |
| Q6 | **已修复**：摘要仍按 200 字节截断，但回退到 UTF-8 边界；post 7 恰在边界，摘要素材逐字节不变 |
| Q13 | **保留**：posts/categories 用 `strconv.Atoi` 语义；pages 用“忽略非数字”的 `parseInt` |
| Q14 | **保留**：未加载的 `category`/`author` 输出零值对象；列表/详情 `tags` 为 `[]`，create/update 未加载时为 `null` |
| Q17 | **保留**：`tags_by_ids`/`all_tags`/`tags_for_posts` 都过滤 `tags.deleted_at`；硬删不级联 `post_tags` |
| Q25 | **保留**：`view_count+1` 同时 `updated_at=NOW(3)`；响应返回读值 +1 |
| Q27 之外的 GetCurrentUser 语义 | **保留**：CreatePost 对缺失/软删用户返回 401 `Unauthorized`；Delete/Restore 对缺失或非 admin 的**库内角色**返回 403 `Permission denied`；公共 pages 的 optional auth 也按库内用户解析 |

### 7.3 GORM 怪癖锁定的行为

| 行为 | 证据 / 测试 |
|---|---|
| `tag_ids: []` 更新会加载**全部**未删除标签并替换关联 | `content_contract::content_write...` 断言 `[1,2,100]` |
| 未传 `tag_ids` 时更新不触碰关联 | 同上，后续 `tag_ids:[2]` 与 `[]` |
| `sort: 0` 无法通过更新写入（`sort > 0` 才赋值） | 实现按 Go 分支 |
| 管理员文章列表不 Preload Author → 零值对象 | `content_read...` 断言 `author.id=0` |
| 管理员列表包含 `is_deleted=true`、排除 `deleted_at` 行 | `total=8`、id 9/4 可见、id 5 为 404 |
| 公共列表分页负 offset 被忽略、`LIMIT 0` 返回空列表 | `page=-1` 返回数据、`page_size=0` 空列表 |
| 页面的 `:id` 以原始字符串绑定 | `GET /_id/abc` → 404；非纯数字 id 的 SQL 强制转换按 MySQL 语义 |

### 7.4 Markdown 管线

- comrak 0.59：`table/strikethrough/autolink/tasklist`、`parse.smart`、`render.unsafe`；随后逐条复刻 `postProcessHTML`（代码块、图片→figure、class、表格清洗、顺序一致）。
- 25 个 Phase 0 golden 全部通过（`services::markdown::tests::golden_corpus_matches_go_baseline`）。
- 规范化后的**已批准差异**只有两类，均是语义等价的表示差异：
  1. 智能标点：goldmark 输出 `&ldquo;` 等实体，comrak 输出 UTF-8 字符；
  2. goldmark Typographer 的 `'twas`/`'em`/`'net` 启发式会把嵌套开引号输出为 `&rsquo;`，comrak 使用 CommonMark 的 `‘`。
- 其余逐字节一致，包括：`<pre class="language-x line-numbers">`、表格 `<thead>/<tbody>` 结构、raw HTML/SVG、HTML 注释、任务列表 checkbox 属性顺序、`![alt](url "")` 的 `title=""`。
- Q6 的摘要截断放在 `services::markdown::excerpt`，Phase 5 的搜索可直接复用。

### 7.5 验证

| 项 | 结果 |
|---|---|
| 单元测试 | 50 个（Markdown golden、slug、excerpt、分页、参数解析等）全部通过 |
| 集成测试 | `tests/content_contract.rs` 两个测试：只读契约 + 写路径/软删/恢复/硬删/表状态 |
| 回归 | Phase 1/2 的 `http_contract`、`auth_contract` 全部通过（原“admin posts 404”占位断言已更新为 200） |
| 质量门禁 | `cargo fmt --all --check`、`cargo clippy --all-targets -D warnings`、`cargo test --all-targets`、`cargo build --locked --release` 全绿 |
| cargo audit | 通过；仅 `.cargo/audit.toml` 已允许的 2 个 unmaintained 警告（bincode/yaml-rust） |
| 搜索索引 | 未接入；Phase 5 在 create/update/delete/restore 提交后更新 tantivy 索引 |

---

## 8. Phase 4 实施记录（2026-11-05）

实现分支：`smileyan-backend-rs` 的 `phase4/comments-subscriptions`。

### 8.1 已实现端点

| 方法 | 路径 | 行为 |
|---|---|---|
| GET | `/api/comments` | 裸数组；仅 `approved` 且 `deleted_at IS NULL`；`post_id` 原始字符串绑定；所有 approved 评论都是顶层（Q18b）；每条挂一层 `children`，children 不按 status 过滤 |
| POST | `/api/comments` | 需登录；`post_id>0`、`content` 非空；Post 只过滤 `deleted_at`（hidden/is_deleted 也可评论）；默认 `pending`；201 + 完整对象 |
| GET | `/api/admin/comments` | `{list,total}`；Atoi 分页；可选 status；`created_at DESC`；Preload User/Post，不加载 Post 关联 |
| POST | `/api/admin/comments/_id/:id/approve` | Atoi id；不存在 → 404；Save 错误忽略后仍 200 `Comment approved` |
| POST | `/api/admin/comments/_id/:id/reject` | 同上，`Comment rejected` |
| DELETE | `/api/admin/comments/_id/:id` | 软删；不存在也 200 `Comment deleted` |
| POST | `/api/subscribe` | `required,email` 校验；active 重复 → 400 `Already subscribed`；inactive → 轮换 token/激活，不发信；新邮箱 → 插入 + 确认邮件 |
| GET | `/api/unsubscribe` | 缺 email → 400 `Invalid parameters`；有 token 校验 email+token，否则仅 email；找不到 → 404；忽略 Save 错误 |
| GET | `/api/admin/subscriptions` | `{list,total}`；Q13 digits-only 分页；软删行排除；`ORDER BY id` |
| DELETE | `/api/admin/subscriptions/_id/:id` | 原始字符串 id 绑定；软删；不存在也 200 |
| POST | `/api/admin/notify` | `post_id` required；Post 只过滤 `deleted_at`；活跃订阅者并发发信（Q9）；响应 `{count,message}` |

### 8.2 决策执行

| ID | 结果 |
|---|---|
| Q9 | **已修复**：`buffer_unordered(5)`，逐收件人记录失败并输出 `sent/failed/total` 汇总；`count` 仍为活跃订阅者数（与 Go 一致），SMTP 失败不影响 200 |
| Q10 | **已修复**：新增可选 `site_url`（默认 `https://smileyan.cn`，`config.yaml` 已写入）；回复链接为绝对 URL；昵称/评论内容/文章标题/摘要做 HTML 转义；默认配置下订阅/通知邮件正文与 Go 夹具逐字节一致 |
| Q13 | **保留**：订阅列表使用 `parse_digits_prefix`（`"1abc"→1`、`"2xyz"→2`），`page_size=0` → 空 list，负 offset 忽略 |
| Q14 | **保留**：评论的 `post`/`user` 始终是完整对象；公共列表/创建使用零值 `Post`；管理员列表 Preload Post 但 Category/Author 为零值、Tags 为 `null` |
| Q18 | **保留**：只挂一层 children；child 自身的 `children` 字段省略 |
| Q18b | **保留**：公共列表不筛选 `parent_id IS NULL`；approved 回复同时作为顶层项与其父的 child 出现（fixture ids `[8,3,2,1]`，8 嵌在 3、3 嵌在 2） |
| Q19 | **保留**：软删邮箱重新订阅 → 唯一索引冲突 → 500 `Failed to subscribe` |

### 8.3 实现要点

- 评论 children 查询为 `parent_id IN (...) AND deleted_at IS NULL ORDER BY id`，没有 status 条件（GORM `Preload("Children")` 的行为）。
- `POST /api/comments` 的当前用户按 `GetCurrentUser` 语义重新查库（id=1 仍短路为合成管理员）；缺失/软删用户 → 401 `Please login first`。
- 回复邮件只在父评论存在（`deleted_at IS NULL`）且父评论作者邮箱 ≠ 当前用户邮箱时发送；父用户缺失时邮箱为空，发信失败被记录并忽略（与 Go 忽略错误一致）。
- 订阅确认邮件只在**新建**订阅时发送；reactivate 路径不发信但轮换 token。
- 通知邮件的正文使用 `site_url` 生成绝对链接；订阅确认与退订链接同样使用 `site_url`。

### 8.4 验证

| 项 | 结果 |
|---|---|
| 单元测试 | 57 个（新增 email 模板/转义、unsubscribe token、`gorm_pagination`、`site_url` 默认值）全部通过 |
| 集成测试 | `tests/comments_contract.rs`（公共/管理员列表、Q18b、创建/回复邮件、审核/拒绝/软删）与 `tests/subscriptions_contract.rs`（分页、订阅/重复/复活/Q19、通知并发与失败、退订 token、管理员删除） |
| 回归 | Phase 1/2/3 的 `http_contract`、`auth_contract`、`content_contract` 全部通过 |
| 质量门禁 | `cargo fmt --all --check`、`cargo clippy --all-targets -D warnings`、`cargo test --all-targets`、`cargo build --locked --release` 全绿 |
| cargo audit | 通过（exit 0）；`.cargo/audit.toml` 忽略 2 条无修复公告，另有 3 条 unmaintained 警告（bincode/yaml-rust/astral-tokio-tar），均为传递依赖 |

---

## 9. Phase 5 实施记录（2026-11-05）

实现分支：`smileyan-backend-rs` 的 `phase5/search`。

### 9.1 端点与索引

| 方法 | 路径 | 行为 |
|---|---|---|
| GET | `/api/search` | `keyword` 必填、空字符串 → 400 `Keyword is required`；`page`/`page_size` 用 `strconv.Atoi`（非法 → 0），默认 1/10；响应 `{list,page,total}`，`list` 无命中为 `null` |

- tantivy 0.27 内存索引，启动时执行 `rebuild`；`AppState` 持有 `Arc<SearchService>`。
- 文档字段：`slug`（删除键，STRING+STORED）、`post_id`/`created_at`（STORED）、`title`/`content`/`excerpt`（jieba search mode + LowerCaser）、`*_uni`（CJK 单字）。
- 查询：`QueryParser` 默认 OR；标题 boost 3.0；未加引号的 CJK 片段展开为单字 `OR`（Bleve standard 的行为）；`title:` 字段查询同时覆盖 `title_uni`；纯负查询（`-Go`）补 `AllQuery`。
- 解析失败保持 Go 的 500 `Search failed`（Q26），不回退为 400。
- 索引写操作在 `spawn_blocking` 中执行；索引失败只记录日志，不影响 HTTP 成功。

### 9.2 决策执行

| ID | 结果 |
|---|---|
| Q1 | **已修复**：更新/删除按旧 slug 操作；软删后 DB slug 变为随机码，索引删除旧 slug；更新 slug 不会留下旧文档 |
| Q2 | **已修复**：启动只索引 `status='published' AND is_deleted=0 AND deleted_at IS NULL`；查询回表后再次过滤 published/未删；草稿、隐藏、`is_deleted`、`deleted_at` 行均不可见 |
| Q6 | **已修复**：搜索摘要按 200 字节截断后回退到 UTF-8 边界，不再产生 `U+FFFD`；长文结尾与 Go 夹具不同（有意） |
| Q26 | **保留**：`keyword="\""` → 500 `Search failed` |
| — | 分页保持 Go 的 `From=(page-1)*page_size`、`Size=page_size`；`page_size=0`、`page=0`、越界均 `list:null`，`total` 不变 |
| — | `search.fallback_like`（默认 true）：索引 `total==0` 时执行 `title LIKE ? OR content LIKE ?`，只查 published/未删；用于切换期兜底 |

### 9.3 验证

| 项 | 结果 |
|---|---|
| 单元测试 | 64 个通过；新增 6 个搜索单元（查询展开、匹配/分页、slug 替换、负查询、非法查询、摘要边界） |
| 集成测试 | `tests/search_contract.rs` 6 个：54 条语料、响应形状、生命周期（create/update/draft/soft-delete/restore/hard-delete）、LIKE 回退、回退开关、空 keyword |
| 语料对比 | 54 条查询在按 Q1/Q2 过滤 Go 命中后**集合完全一致**；top-10 重叠 100%；分页 `total` 与列表长度一致 |
| 泄漏检查 | `draft-post`、`hidden-post`、`deletedpost`、`published-but-is-deleted`、`soft-deleted-at-post` 均不出现；`zzqpubdel` 返回空 |
| 回归 | Phase 1/2/3/4 的 `http_contract`、`auth_contract`、`content_contract`、`comments_contract`、`subscriptions_contract` 全部通过 |
| 质量门禁 | `cargo fmt --all --check`、`cargo clippy --all-targets -D warnings`、`cargo test --all-targets`、`cargo build --locked --release`、`cargo audit`（exit 0）全部通过 |

### 9.4 已知差异

- Lucene 风格 `zzq*` 前缀查询暂不支持（tantivy 只支持 `"phrase"*`）。语料中该查询在 Go 下只命中隐藏/草稿/已删除文章，Q2 修复后正确结果本应为空，因此不影响语料验收；Phase 7 再评估是否补 `PrefixQuery`。
- 搜索摘要只做 UTF-8 边界截断，不做 Markdown 清洗（与 Go 一致）；因此长文摘要比 Go 少一个 `U+FFFD` 且可能短 1–2 字节，这是 Q6 的批准差异。

---

## 10. Phase 6 实施记录（2026-11-05）

实现分支：`smileyan-backend-rs` 的 `phase6/tts-uploads`。

### 10.1 端点

| 方法 | 路径 | 认证 | 行为 |
|---|---|---|---|
| POST | `/v1/audio/speech` | Bearer JWT（Auth，不要求 Admin） | OpenAI 兼容 TTS 代理；请求校验 → 上游 `POST {base_url}/v1/audio/speech` → 二进制或 SSE 透传 |
| POST | `/api/upload-avatar` | Bearer JWT | 头像上传；代码移到 `handlers/uploads.rs`，行为与 Phase 2 一致 |
| GET | `/uploads/*` | 无 | `ServeDir` 提供上传目录；不列目录，缺失文件 404 |

- `/v1` 与 `/api` 是 Gin 中的两个独立认证组；Rust 用单独的嵌套 Router + `auth_middleware`，未挂 Admin。
- `AppState.http` 的 `connect_timeout` 为 10 s；非 SSE 请求额外设置 120 s 总超时（reqwest 的 total timeout 覆盖响应体读取），SSE 不设总超时。
- 二进制响应：`Content-Type: application/octet-stream`、`Content-Disposition: attachment; filename=speech.<format>`，body 不缓冲。
- SSE 响应：`Content-Type: text/event-stream`、`Cache-Control: no-cache`、`Connection: keep-alive`，body 不缓冲。
- 上游非 200：返回相同状态码 + `{"error":"TTS service error","detail":"<body>"}`；读取 body 最多 64 KiB。
- 上游连接失败/超时：500 `{"error":"Failed to call TTS service"}`。
- 未配置 `tts.base_url`/`tts.api_key`：500 `{"error":"TTS service not configured"}`，日志只记录两个布尔值。

### 10.2 决策执行

| ID | 结果 |
|---|---|
| Q11 | **已修复**：`input`/`instructions` 按字符数 4096 校验（Go 按字节）；非 SSE 120 s 总超时 + 10 s connect timeout，SSE 无总超时。1366 个汉字（4098 字节）从 Go 的 400 变为成功，这是批准差异。 |
| Q8 | **保留 Phase 2 实现**：`.jpg/.jpeg/.png/.gif` 大小写不敏感、`upload.max_size` 生效、文件名去路径/控制字符；上传代码移入 `handlers/uploads.rs`。 |
| Q16 | **已修复**：启动时 `create_dir_all(upload.path)` 与 `create_dir_all(upload.avatar_path)`（Phase 1 已实现）；Phase 6 增加静态文件 200/404 覆盖。 |
| Q27 | **保留**：id=1 合成管理员上传文件成功但 `users.avatar` 不更新。 |

### 10.3 验证

| 项 | 结果 |
|---|---|
| 单元测试 | 72 个通过；Phase 6 新增 8 个 TTS 单元（全部校验文案、Q11 字符数、默认值/voice 对象、未配置 500、`null` 字符串字段、连接拒绝、上游 drop、超时）；`sanitize_filename` 测试随代码移动 |
| 集成测试 | `tests/tts_contract.rs` 2 个：认证/所有校验分支、`/v1` 方法不匹配/未知路径 404、wiremock 二进制字节、SSE 原文与 headers、上游 400/500 透传、上游请求体/Authorization、Q11 1366 汉字成功、未配置 500；`tests/auth_contract.rs` 追加缺失静态文件 404 与目录不可列 |
| 回归 | Phase 1–5 的 `http_contract`、`auth_contract`、`content_contract`、`comments_contract`、`subscriptions_contract`、`search_contract` 全部通过 |
| 质量门禁 | `cargo fmt --all --check`、`cargo clippy --all-targets -D warnings`、`cargo test --all-targets`、`cargo build --locked --release`、`cargo audit`（exit 0）全部通过 |

### 10.4 已知差异

- **Q11（批准）**：`tts_input_cjk_byte_quirk` 夹具（1366 个汉字 → 400）不再逐字节匹配；Rust 按文档字符数接受该请求。`tts_binary` 的 1365 个汉字仍为 200。
- 上游错误 body 最多读取 64 KiB（Go 无上限）；正常错误体不受影响。
- 发给上游的请求头与 Go 默认 transport 不完全相同：Go 会带 `Accept-Encoding: gzip` / `User-Agent: Go-http-client/1.1`，reqwest 不带 gzip（未启用 gzip feature）并使用自己的 UA。上游只依赖 `Content-Type`/`Authorization` 与 JSON body；API 响应不变。
- 非 SSE 的 120 s 总超时是新增保护；Go 无超时。超时后若响应已开始，客户端会看到截断的音频流（与 Go 上游中断时一致）。
- `release.yml`/`deploy-remote.sh` 的 TTS 三个变量转发**推迟到 Phase 8**：Rust 仓库尚无 release workflow，Phase 8 创建时一并加入；`config.yaml` 与 `.env.example` 已包含 `tts` 段/变量。

---

## 11. Phase 7 实施记录（2026-11-05）

实现分支：`smileyan-backend-rs` 的 `phase7/contract-perf-security`。
目标：全量契约、Markdown/搜索验收、Go vs Rust 压测、依赖审计、真实前端联调、
部署/回滚文档。

### 11.1 全量契约重放

`tests/contract_full.rs` 按 `tests/fixtures/contract/index.json` 的采集顺序重放：

| 类型 | 数量 | 校验内容 |
|---|---:|---|
| HTTP | 172 | 状态码、夹具中的响应头（跳过 `connection`/`transfer-encoding`）、JSON/文本/二进制 body |
| 邮件 | 8 | 收件人、标题、正文（验证码/退订 token 归一化；Q10 回复链接按绝对 URL 比较） |
| note | 13 | Redis 快照（exists/TTL class/IP 计数）、TTS 上游 body、创建 id、时间偏移 |

归一化与采集脚本一致：`created_at/updated_at/deleted_at` → `<timestamp>`、
JWT → `<jwt>`、32-hex → `<token>`、8-hex slug → `<random-slug>`；
搜索列表按 id 排序后逐项比较（相关性顺序不是契约），搜索结果中
`published-long` 的 `excerpt` 统一为 `<q6-excerpt>`（Q6）。

只对已批准差异做显式调整：

| 项 | 调整 |
|---|---|
| Q2b | 公共文章列表剔除 `is_deleted` 行并重算 `total`；page 2 用 Go 全序重算；post 9 的 `view_count` 9 → 8（泄漏详情不再自增） |
| Q4 | 公共页面列表只保留 published，`total` 重算 |
| Q1/Q2/Q6 | 草稿/隐藏/已删除搜索结果为 0；删除/恢复后的 `total` 与命中集合按新语义；长文摘要哨兵 |
| Q7 | `login_short_email_panic` 500→200；`newuser` 昵称/id 顺延为 101；`admin_users` 插入 `a@b.co` |
| Q8 | `upload_avatar_jpeg_ext` 400→200 |
| Q10 | 回复邮件链接改为绝对 URL |
| Q11 | 1366 汉字 TTS 请求 400→200 二进制 |
| Q20 | HS512 管理员 token 200→401 |

未调整的 160+ 个夹具逐字段一致，包括 401/403/404/429 文案、GORM 零值对象、
`tags: null`/`[]`、Q13 分页怪癖、Q18b 顶层重复、Q25 `updated_at`、时间格式。

### 11.2 Markdown / 搜索

- Markdown golden：25 个 corpus 用例（含 `docs/markdown-example.md`）逐字节通过。
  重新验证了关闭 comrak 默认 feature 后输出不变。
- 搜索：54 条语料在 Q1/Q2 过滤后命中集合与 Go 完全一致；`tests/search_contract.rs`
  6 个测试全绿。已知差异仅为相关性顺序（top-10 集合 100% 重合）和
  `page_size=1` 的分页内容；见 `docs/api-differences.md`。

### 11.3 性能（Apple M5 / MySQL 8.4 / Redis 7.4，`oha -n 3000 -c 50`）

| 端点 | Go p95 | Rust p95 | 比值 |
|---|---:|---:|---:|
| `GET /api/posts?page=1&page_size=10` | 26.82 ms | 22.37 ms | 0.83× |
| `GET /api/posts/{slug}` | 129.15 ms | 60.53 ms | 0.47× |
| `GET /api/search?keyword=文` | 58.82 ms | 16.66 ms | 0.28× |
| 峰值 RSS | 44.9 MB | 61.3 MB | 1.37× |

计划目标（p95 ≤ 1.2×/1.5×，RSS ≤ 2×）全部达标。脚本：`scripts/bench.sh`；
完整数据：`docs/performance.md`。

### 11.4 依赖与安全

- `cargo audit` exit 0：仅 `astral-tokio-tar`（testcontainers dev 依赖）的
  unmaintained 警告。
- 新增 `deny.toml`：`cargo deny check` 的 advisories/bans/licenses/sources
  全部通过；只允许宽松许可证，禁止 GPL/未知许可证、git/未知 registry、wildcard。
- `comrak` 改为 `default-features = false`，移除未使用的 `cli`/`syntect-onig`
  依赖树（`bincode`/`yaml-rust`/onig），Markdown golden 仍全绿。
- 完整 checklist 与剩余项（上传魔数、可信代理、2 MiB body 上限、`user_id=1`
  短路）见 `docs/security-checklist.md`。

### 11.5 真实前端联调

Vite dev server（5173）→ Rust（8080，真实 MySQL/Redis/MailHog）用真实 Vue 应用完成：

1. 管理员通过邮箱验证码登录（MailHog 收信，Redis 取码）；
2. `GET /api/posts`、分类/标签、文章详情、评论列表正常渲染；
3. 新建文章（Typora 源码模式）→ 发布 → 列表出现；再次编辑标题 → 保存；
4. 评论提交（pending）→ 管理后台审核通过 → 公共文章页显示；
5. 搜索新文章命中；`/api/search` 的列表/`total` 正常。

TTS：前端使用浏览器 `speechSynthesis`，没有调用 `/v1/audio/speech`；该端点由
`tests/tts_contract.rs`（wiremock 二进制/SSE/错误）与全量夹具覆盖。

### 11.6 测试稳定性修复

- 增加 MySQL/Redis 就绪探针：MySQL 8.4 初始化期间会短暂开放端口再重启，Redis
  可能在端口开放后仍在加载；现在等待真实 `SELECT 1`/`PING` 后再建连接池。
- 连接地址固定为 `127.0.0.1`：`get_host()` 返回 `localhost` 时可能优先解析
  `::1`，导致 Docker Desktop 的端口映射间歇性不可达。
- 测试容器改为固定映射端口 **23306/26379**（低于临时端口范围，可用
  `TEST_MYSQL_PORT`/`TEST_REDIS_PORT` 覆盖），并用跨进程文件锁
  （`/tmp/smileyan-backend-rs-containers.lock`）串行化所有测试二进制；此前
  `cargo test --all-targets` 会并行启动十余个 MySQL，且随机高位端口会与
  TIME_WAIT 套接字冲突，导致连接挂起。
- 连续 6 次 `auth_contract`、3 次 `http_contract`、多次全量 `cargo test` 通过。

### 11.7 门禁与文档

- `cargo fmt --all --check`、`cargo clippy --all-targets -D warnings`、
  `cargo test --all-targets`（72 单元 + 全部集成）、`cargo build --locked --release`、
  `cargo audit`、`cargo deny check` 全绿。
- 新增文档：`docs/api-differences.md`、`docs/performance.md`、
  `docs/security-checklist.md`、`docs/deployment.md`；README 增加 Phase 7 段。
- 部署/回滚完整步骤见 `docs/deployment.md`；musl 制品、staging、灰度与
  `deploy-remote.sh` 留给 Phase 8。

## 12. Phase 8 实施记录：部署与切换（2026-11-05）

分支 `phase8/deploy-cutover`。API/数据库/Redis/JWT 契约未变；本节只记录部署
与运维相关变更。

| 项 | 结果 |
|---|---|
| release workflow | `cross` 构建 `x86_64-unknown-linux-musl`；`file` 接受 `statically linked`/`static-pie linked`，`readelf -d` 必须无 `NEEDED`（`ldd` 对 static-pie 退出 0，不可用）；80 MiB 上限；三件套清单 + `.env` 泄漏检查；stable tag 自动部署 |
| staging workflow | 手动 `workflow_dispatch` 输入 tag；`STAGING_SERVERS_CONFIG`；复用 prod tarball，部署到 8081；部署后运行 `smoke.sh` |
| `deploy-remote.sh` | 原子 `.env`（TTS 三变量仅非空写入）；PID 命令行校验后停止；无 PID 时回退旧 `stop.sh`；解压前备份 `smileyan-backend.prev`/`config.yaml.prev`；生成 `rollback.sh`；`APP_PORT` 改写 `server.port`；30 s 健康检查 |
| 进程隔离 | 生成的 `stop.sh` 只按本目录 PID + 本端口（lsof，Linux 回退 fuser），不再 `pkill -f`；staging 8081 与生产 8080 可同机 |
| 回滚 | `rollback.sh` 停当前 → 失败二进制留存 `.failed` → 恢复 `.prev`/`config.yaml.prev` → 启动 + 健康检查；`.prev` 保留可重试 |
| 本地脚本 | `bin/start|stop|restart|dev`；`bin/build-and-package.sh` 输出 dev/prod/staging 三件套（staging 8081） |
| 冒烟 | `scripts/smoke.sh` 13 项：hello/categories/tags/posts/pages/comments/search/uploads 404，token 可选 |
| 回滚演练 | 真实二进制 + MySQL 8.4/Redis 7.4：deploy v1 → deploy v2 → rollback，2 s；sha256 恢复、旧 JWT `/api/user` 200、`/api/posts` 可读、`/api/search` 命中（重启重建索引） |
| 门禁 | fmt/clippy/test（72 单元 + 全部集成）/release build 通过；`cargo audit --no-fetch`、`cargo deny --offline check` 通过（网络受限，使用本地 advisory DB；依赖未变） |
| 未执行 | CI 的 musl 构建（首次 tag 触发）、真实 staging/生产部署、7 天观察、Go 归档 |

**不变量**：数据库结构、Redis key/TTL、JWT secret 与 claims、`config.yaml`
旧字段均未改变；回滚不需要数据迁移。

## 13. 新增接口：`GET /api/posts/dates`（2026-11-10，方案 A）

首页日历原先在 `smileyan-frontend/src/views/Home.vue` 用
`GET /api/posts?page_size=1000` 拉全量文章（含 `content`/`html_content`），
与列表请求并行发出：1.787 MB / 17 s，且超过 1000 篇后静默漏日期。方案 A 新增
只读日期聚合端点，**不改动任何既有端点契约**。

| 项 | 内容 |
|---|---|
| 路径 | `GET /api/posts/dates`（公共、无认证） |
| 响应 | 裸数组，按日期倒序：`[{"date":"2026-11-09","count":2}, ...]`；空库为 `[]` |
| 过滤 | `deleted_at IS NULL AND status='published' AND is_deleted=0 AND created_at IS NOT NULL` |
| 时区 | `DATE(created_at)` 按数据库存储的墙钟日期分组，与 `server.timezone` 及前端 `dayjs(created_at)` 一致，不做 UTC 转换 |
| 缓存 | `Cache-Control: public, max-age=60`；前端另有模块级 Promise 缓存（失败不缓存） |
| 路由 | 静态段 `/posts/dates` 注册在 `/posts/{slug}` 之前；若线上存在 slug=`dates` 的文章会被抢占，上线前执行 `SELECT 1 FROM posts WHERE slug='dates'` 检查 |
| 索引 | 不新增；现有 `idx_posts_status`/`idx_posts_is_deleted` 足够博客量级 |
| 实现 | `repo/posts.rs::list_dates`（`PostDateCountRow`）+ `handlers/posts.rs::get_post_dates` + `PostDateCountDto` |
| 测试 | `tests/post_dates_contract.rs`：仅 published、同日合并计数、草稿/隐藏/`is_deleted`/软删不计入、空库 `[]`、NULL 时间戳、跨零点不偏移、Content-Type |

前端改动：`Home.vue` 的 `loadPostDates` 改调新接口；模块级 `postDatesPromise`
在 SPA 生命周期内复用，从文章页返回首页不再重复请求；`page_size: 1000` 已移除。

回滚：前端恢复旧的 `loadPostDates` 即可（会重新出现双请求）；后端新增路由为
增量，可保留。发布顺序仍建议后端先行，前端先上线遇 404 时日历降级为无圆点。

## 14. 新增接口：`GET /api/posts/summary`（2026-11-10，首页提速）

首页列表原先调用 `GET /api/posts?page=1&page_size=10`，返回整篇文章正文与作者：
10 篇约 358 KB（全量 188 篇约 1.8 MB），且每次请求串行执行 5 条 SQL
（count、list、categories、authors、tags）。前端卡片实际只读
`id/title/slug/excerpt/cover_image/created_at/view_count/category.name/tags[].{id,name}`。
新增只读卡片端点，**不改动任何既有端点契约**。

| 项 | 内容 |
|---|---|
| 路径 | `GET /api/posts/summary`（公共、无认证） |
| 查询参数 | 与 `GET /api/posts` 完全一致：`page`、`page_size`、`category_id`、`tag_id`、`keyword`、`start_date`、`end_date`；分页 quirks（`page=abc → 0`、`page_size=0`、负 limit/offset）保持一致 |
| 响应 | `{list,page,total}`；item：`{id,created_at,title,slug,excerpt,cover_image,view_count,category,tags}` |
| 省略字段 | `content`、`html_content`、`author`、`updated_at`、`status`、`is_deleted`、`category_id`、`author_id` |
| 关联 | `category` 为 `{id,name}` 或 `null`（分类缺失/软删）；`tags` 恒为 `[]`/数组，无 `null` |
| 摘要 | 沿用公共列表规则：存储的 `excerpt` 为空时，用 `LEFT(content,600)` 前缀经 `markdown::excerpt` 生成；仅对该页中摘要为空的行补一条查询，常规页不读正文 |
| SQL | 3 条：`count` 与 `list_summary` 并发（`tokio::try_join!`），随后 1 条 tag 查询；分类名通过 `LEFT JOIN categories ... AND deleted_at IS NULL` 随主查询返回，不再单独查分类/作者 |
| 载荷 | 实测 10 篇 4.7 KB（原 358 KB，约 1/76）；本机 curl 50–105 ms（原 305–530 ms） |
| 路由 | 静态段 `/posts/summary` 注册在 `/posts/{slug}` 之前；若线上存在 slug=`summary` 的文章会被抢占，上线前执行 `SELECT 1 FROM posts WHERE slug='summary'` 检查 |
| 索引 | 不新增；沿用 `idx_posts_status`/`idx_posts_is_deleted`/`idx_posts_category_id` |
| 实现 | `repo/posts.rs::{list_summary,summary_tags_for_posts,content_previews}` + `handlers/posts.rs::get_posts_summary` + `PostSummaryDto` |
| 测试 | `tests/post_summary_contract.rs`：卡片字段集合（无正文/作者）、与 `GET /api/posts` 逐字段一致、`created_at DESC` 顺序、草稿/隐藏/`is_deleted`/软删排除、`category=null`、软删标签不返回、空摘要回退、筛选与分页 quirks、空库 `list=[]`、Content-Type |

前端改动：`Home.vue` 的 `loadPosts` 与 `loadPostsByDate` 改调新接口，字段读取不变；
`AdminSubscriptions.vue` 的文章下拉仍用 `GET /api/posts`。

回滚：前端改回 `/api/posts` 即可；后端新增路由为增量，可保留。

## 15. 新增接口：`GET /api/admin/posts/summary`（2026-11-10，后台列表提速）

后台文章管理页原先调用 `GET /api/admin/posts?page=1&page_size=20`，每行都带
`content` 与 `html_content`（整篇文章正文），20 行即可达数百 KB；并且每次请求
串行执行 5 条 SQL（count、list、categories、authors、tags），而页面表格只读
`id/title/slug/status/view_count/created_at`。新增只读表格端点，**不改动
`GET /api/admin/posts` 的任何契约**。

| 项 | 内容 |
|---|---|
| 路径 | `GET /api/admin/posts/summary`（管理员认证，与 `/api/admin/posts` 同级） |
| 查询参数 | 与 `GET /api/admin/posts` 完全一致：`page`、`page_size`、`status`、`category_id`、`tag_id`、`keyword`、`start_date`、`end_date`；分页 quirks（`page=abc → 0`、`page_size=0`、负 limit/offset）保持一致 |
| 响应 | `{list,page,page_size,total}`；行：`{id,created_at,title,slug,status,view_count}` |
| 省略字段 | `content`、`html_content`、`excerpt`、`cover_image`、`category`、`tags`、`author`、`is_deleted`、`updated_at`、`category_id`、`author_id` |
| SQL | 2 条：`count` 与 `list_admin_summary` 并发（`tokio::try_join!`）；不预加载任何关联 |
| 路由 | 静态段 `/posts/summary` 与 `/posts/slug/{slug}`、`/posts/_id/{id}` 同为固定前缀，无参数段冲突，不占用 slug |
| 索引 | 不新增；沿用 `idx_posts_deleted_at`/`idx_posts_status`/`idx_posts_is_deleted`/`idx_posts_category_id` |
| 实现 | `repo/posts.rs::list_admin_summary` + `handlers/posts.rs::get_admin_posts_summary` + `AdminPostSummaryDto` |
| 测试 | `tests/admin_post_summary_contract.rs`：401/403、行字段集合（无正文/关联）、与 `GET /api/admin/posts` 的 id/total/page/字段一致性、筛选与分页 quirks、空库 `list=[]`、Content-Type |

前端改动：`AdminPosts.vue` 的 `loadPosts` 改调新接口；分类、标签、文章三个请求
并行发起（与首页一致），不再 `await` 分类/标签后再拉文章列表。

回滚：前端改回 `/api/admin/posts` 即可；后端新增路由为增量，可保留。
