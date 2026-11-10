# 前端实际 API 使用核查

> Phase 0 交付物 · 2026-11-04
> 前端仓库：`../smileyan-frontend`（Vue 3 + axios）
> 后端基线：`smileyan-backend` @ `go-baseline-2026-11`
>
> 目的：`docs/api.md` 的“通用 `{data:...}` 包装”与实际代码不符。本文件以**真实前端调用**为准，
> 冻结 Rust 重写必须满足的路径、请求字段与响应字段。
>
> 2026-11-10 更新：首页日历改用新增的 `GET /api/posts/dates`（方案 A），
> 不再用 `GET /api/posts?page_size=1000` 拉取全量文章。

## 1. 基础配置

- `axios.defaults.baseURL = VITE_API_BASE_URL || ''`，timeout 30s（`src/utils/axios.js:6-7`）。
- 开发代理只转发 `/api` 到 `BACKEND_TARGET`（`vite.config.js:10,19-27`）。
- 生产由 Nginx/Caddy 反代 `/api/*`；`/v1`、`/uploads` 没有文档化代理规则。
- `Authorization: Bearer <localStorage.token>` 由 axios 全局注入（`src/store/user.js:5-8,25`）。
- 所有成功响应都是**裸对象/数组**，没有 `{data:...}` 包装；错误体统一 `{"error":"..."}`。

## 2. 端点与请求形状

| 方法 | 路径 | 调用位置 | 参数 / Body |
|---|---|---|---|
| POST | `/api/send-code` | `App.vue:154` | `{email}` |
| POST | `/api/login` | `App.vue:174` | `{email, code}` |
| GET | `/api/user` | `store/user.js:36` | 无 |
| PUT | `/api/user` | `App.vue:203` | `{nickname}`（从不发送 avatar） |
| GET | `/api/posts/dates` | `Home.vue:192` | 无 |
| GET | `/api/posts` | `Home.vue:211` | `{page, page_size, category_id?, tag_id?}` |
| GET | `/api/posts` | `Home.vue:218` | `{page:1, page_size:100, start_date, end_date}` |
| GET | `/api/posts` | `AdminSubscriptions.vue:82` | `{page_size:100}`（管理员 token） |
| GET | `/api/posts/:slug` | `Post.vue:306` | path |
| GET | `/api/categories` | `Home.vue:181`、`AdminPosts.vue:140`、`Editor.vue:784`、`AdminCategories.vue:64` | 无 |
| GET | `/api/tags` | `Home.vue:186`、`AdminPosts.vue:149`、`Editor.vue:793`、`AdminTags.vue:52` | 无 |
| GET | `/api/pages` | `App.vue:138` | 无 |
| GET | `/api/pages` | `AdminPages.vue:89` | `{page, page_size}` |
| GET | `/api/pages/:slug` | `Page.vue:31` | path |
| GET | `/api/comments` | `Post.vue:340` | `{post_id}` |
| POST | `/api/comments` | `Post.vue:352` | `{post_id, content, parent_id}` |
| GET | `/api/search` | `Home.vue:245` | `{keyword, page, page_size}` |
| GET | `/api/admin/posts` | `AdminPosts.vue:163` | `{page, page_size, status?, category_id?, tag_id?, keyword?}` |
| GET | `/api/admin/posts/_id/:id` | `Editor.vue:802` | path |
| POST | `/api/admin/posts` | `Editor.vue:826` | `{title, slug, content, excerpt, cover_image, category_id, tag_ids, status}` |
| PUT | `/api/admin/posts/_id/:id` | `Editor.vue:834` | 同上 |
| DELETE | `/api/admin/posts/_id/:id` | `AdminPosts.vue:179` | path |
| GET | `/api/admin/comments` | `AdminComments.vue:71` | `{page, page_size, status?}` |
| POST | `/api/admin/comments/_id/:id/approve` | `AdminComments.vue:78` | `{}` |
| POST | `/api/admin/comments/_id/:id/reject` | `AdminComments.vue:88` | `{}` |
| DELETE | `/api/admin/comments/_id/:id` | `AdminComments.vue:99` | path |
| POST | `/api/admin/categories` | `AdminCategories.vue:93` | `{name, slug, description, sort}` |
| PUT | `/api/admin/categories/_id/:id` | `AdminCategories.vue:91` | 同上 |
| DELETE | `/api/admin/categories/_id/:id` | `AdminCategories.vue:80` | path |
| POST | `/api/admin/tags` | `AdminTags.vue:69` | `{name, slug}` |
| DELETE | `/api/admin/tags/_id/:id` | `AdminTags.vue:59` | path |
| POST | `/api/admin/pages` | `AdminPages.vue:125` | `{title, slug, content, status}` |
| PUT | `/api/admin/pages/_id/:id` | `AdminPages.vue:123` | 同上 |
| DELETE | `/api/admin/pages/_id/:id` | `AdminPages.vue:112` | path |
| GET | `/api/admin/subscriptions` | `AdminSubscriptions.vue:74` | `{page, page_size}` |
| DELETE | `/api/admin/subscriptions/_id/:id` | `AdminSubscriptions.vue:91` | path |
| POST | `/api/admin/notify` | `AdminSubscriptions.vue:109` | `{post_id}` |

**路径全部存在**于 `routes/routes.go`，没有调用未注册端点。

前端**未调用**的 Go 端点：

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/hello` | 仅健康检查 |
| POST | `/api/subscribe` | 订阅入口不在当前 Vue 应用 |
| GET | `/api/unsubscribe` | 邮件链接直接访问 |
| POST | `/api/upload-avatar` | 模板中无 `<el-upload>`，属于死代码 |
| POST | `/v1/audio/speech` | Post.vue 使用浏览器 `speechSynthesis` |
| GET | `/api/admin/posts/slug/:slug` | 未使用 |
| POST | `/api/admin/posts/_id/:id/restore` | 未使用 |
| GET | `/api/admin/pages/_id/:id` | 未使用 |
| GET | `/api/admin/users` | 未使用 |
| PUT | `/api/admin/users/:id/role` | 未使用 |

## 3. 响应字段（前端实际读取）

| 端点 | 读取字段 |
|---|---|
| POST `/api/login` | `token`、`user.{id,email,nickname,avatar,role}`；`user.role === 'admin'` |
| GET/PUT `/api/user` | 整个 body 当 user 对象：`role,nickname,avatar,email` |
| GET `/api/pages`（`App.vue`） | 当作**数组**：`{id,slug,title}` |
| GET `/api/pages`（`AdminPages`） | `list[].{id,title,slug,content,status,created_at}`、`total` |
| GET `/api/pages/:slug` | `title`、`html_content` |
| GET `/api/posts` | `list`、`total`；item：`id,slug,title,cover_image,created_at,category.name,view_count,excerpt,highlights?[0],tags[].{id,name}`；管理员视图还读 `status` |
| GET `/api/posts/dates` | 裸数组；`[].{date,count}`（日历当前只读 `date`） |
| GET `/api/posts/:slug` | `id,title,created_at,category.name,view_count,tags[],html_content,content` |
| GET `/api/categories` | 裸数组；`id,name,post_count`；管理页读 `slug,description,sort` |
| GET `/api/tags` | 裸数组；`id,name,post_count` |
| GET `/api/comments` | 裸数组；`id,created_at,content,user.{avatar,nickname},children[].{id,created_at,content,user.{avatar,nickname}}` |
| POST `/api/comments` | 响应被忽略 |
| GET `/api/search` | `list`、`total`；与文章卡片相同字段；`highlights?.[0]` 可选 |
| GET `/api/admin/posts` | `list`、`total`；行：`id,title,slug,status,view_count,created_at` |
| GET `/api/admin/posts/_id/:id` | `id,title,slug,content,excerpt,cover_image,category_id,tags[].id,status` |
| POST `/api/admin/posts` | `id`（兜底 `_id`） |
| GET `/api/admin/comments` | `list`、`total`；行：`id,user?.{nickname,email},content,post?.{title},status,created_at` |
| GET `/api/admin/subscriptions` | `list`、`total`；行：`id,email,is_active,created_at` |
| 其余写操作 | 响应被忽略 |

前端**不读取**：`author`、`is_deleted`、`updated_at`、`parent`、响应中的 `page`/`page_size`。

## 4. Rust 重写必须保留的契约

1. **没有 `{data:...}` 包装**。所有成功响应保持裸对象/数组。
2. **`list` 不能是 `null`**：多个调用点无空值保护地 `.forEach`/`.filter`。Go 的空结果行为需逐端点用夹具确认；Rust 对前端实际消费的列表输出 `[]` 更安全，但契约测试要以 Go 夹具为准并记录已批准差异。
3. **`/api/categories`、`/api/tags`、`/api/comments`、`/api/posts/dates` 是裸数组**，不要统一为分页对象。
4. **`comment.user` 必须是对象**，即使 Preload 缺失或用户被软删；保持 Go 的零值 `User` 形状。
5. **`post.category` 未 Preload/分类软删时是零值对象**，不是 `null`（Go 的 `omitempty` 对结构体无效）。Rust 若要改 `null`，需前端确认并记录差异。
6. **`post_count` 必须存在**，且只统计 `published AND is_deleted=0`。
7. **日期字符串**必须可被 `new Date()`/dayjs 解析；保持 RFC3339 本地偏移，不输出 epoch 数字。
8. **枚举小写**：`admin|reader`、`published|draft|hidden`、`pending|approved|rejected`。
9. **错误体** `{"error":"..."}`。
10. **`/uploads/...` 由后端提供**；`cover_image`、`avatar`、`html_content` 中的 URL 原样返回。
11. **创建文章返回 `id`**（前端有 `_id` 兜底，但不要依赖）。
12. **approve/reject 接受 `{}`** 空 JSON body。
13. **公共 `GET /api/posts` 对管理员也返回 published**（公共路由无 Auth）；不要改成“管理员可见草稿”，否则会改变 `AdminSubscriptions` 的文章下拉。
14. **`GET /api/pages` 的双形状矛盾冻结为 Go 行为**（`{list,total}`）；`App.vue` 的数组假设是前端 bug，不在后端重写中修复。

## 5. 已知前端问题（记录，不在本次修复）

| # | 问题 | 位置 |
|---|---|---|
| F1 | `App.vue` 把 `GET /api/pages` 的 `{list,total}` 当数组，页面导航静默为空 | `App.vue:139`、模板 `:16-18` |
| F2 | `AdminPages.vue` 直接 import 原生 `axios`，绕过 `@/utils/axios` 的全局配置 | `AdminPages.vue:67` |
| F3 | 头像上传 UI 不存在，`handleAvatarSuccess` 是死代码 | `App.vue:197-199,382-399` |
| F4 | 开发代理只覆盖 `/api`，`/v1`、`/uploads` 未代理 | `vite.config.js:19-27` |
| F5 | 前端把 `highlights?.[0]` 当摘要兜底，但后端从未返回 | `Home.vue:38` |
