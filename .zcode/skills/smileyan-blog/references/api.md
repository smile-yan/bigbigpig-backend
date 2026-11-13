# Smileyan 博客 API 参考（API 令牌视角）

本文只描述 `scripts/blog.py` 用到的端点、字段和已验证的后端怪癖。
基线是 `bigbigpig-backend` 当前 `main`（令牌功能 2026-11-13）。

## 1. 认证与环境

- 令牌来自博客后台「令牌管理」，格式 `sk_` + 64 位小写十六进制，共 67 字符。
- 有效期 1–365 天；数据库只存 SHA-256，明文只在创建响应中出现一次。
- 请求头二选一：
  - `Authorization: Bearer sk_...`（推荐；CORS 白名单只含 `Authorization`）
  - `X-API-Key: sk_...`
- 环境变量：
  - `SMILEYAN_BLOG_TOKEN`：令牌本身
  - `SMILEYAN_BLOG_URL`：默认 `http://127.0.0.1:8080`
- 如果当前进程没有这两个变量，CLI 会按 `~/.zshenv`、`~/.zprofile`、`~/.zshrc`、
  `~/.bash_profile`、`~/.bashrc`、`~/.profile` 的顺序只读扫描同名 `export` 行；
  不执行文件、不打印值。把变量放进 `~/.zshenv` 可以让非交互 zsh 也稳定继承。

- 令牌只允许 `GET` / `POST` / `PUT`。任何 `DELETE` 都会落回管理员 JWT
  校验，用令牌调用会得到 401 `{"error":"Invalid or expired token"}`。

> 2026-11-13 检查：`https://smileyan.cn/api/*` 由 Caddy 返回 404；
> `http://smileyan.cn:8080` 是旧后端，没有 `/api/posts/summary` 和令牌接口。
> 令牌功能目前只在本机新后端可用。线上部署/代理完成后，用
> `SMILEYAN_BLOG_URL` 指向实际 API 地址，并先跑 `check`。

## 2. 权限矩阵

### 令牌可用（后台内容路由）

| 方法 | 路径 | 用途 |
|---|---|---|
| GET | `/api/admin/posts/summary` | 轻量文章表格行 |
| GET | `/api/admin/posts` | 完整文章列表（含正文、分类、标签） |
| GET | `/api/admin/posts/_id/{id}` | 按 ID 读完整文章（含作者） |
| GET | `/api/admin/posts/slug/{slug}` | 按 slug 读完整文章（含作者，可读草稿） |
| POST | `/api/admin/posts` | 创建文章 |
| PUT | `/api/admin/posts/_id/{id}` | 更新文章 |
| GET | `/api/admin/pages` | 页面列表（含草稿、正文） |
| GET | `/api/admin/pages/_id/{id}` | 按 ID 读页面 |
| POST | `/api/admin/pages` | 创建页面 |
| PUT | `/api/admin/pages/_id/{id}` | 更新页面 |
| POST | `/api/admin/tags` | 创建标签 |
| PUT | `/api/admin/tags/_id/{id}` | 更新标签 |
| POST | `/api/admin/categories` | 创建分类 |
| PUT | `/api/admin/categories/_id/{id}` | 更新分类 |
| GET | `/api/admin/comments` | 后台评论列表 |
| POST | `/api/admin/comments/_id/{id}/approve` | 通过审核 |
| POST | `/api/admin/comments/_id/{id}/reject` | 拒绝审核 |

### 令牌不可用（管理员 JWT 专属）

- 所有 `DELETE`（文章、页面、评论、标签、分类、订阅、令牌）
- `GET /api/admin/users`、`PUT /api/admin/users/{id}/role`
- `POST /api/admin/posts/_id/{id}/restore`
- `GET /api/admin/subscriptions`、`DELETE /api/admin/subscriptions/_id/{id}`
- `POST /api/admin/notify`
- `GET/POST /api/admin/keys`、`PUT/DELETE /api/admin/keys/_id/{id}`
- `POST /api/upload-avatar`、`GET/PUT /api/user`、`POST /api/comments`
- `POST /v1/audio/speech`

用令牌访问这些路由会得到 401（不是 403），因为中间件把 `sk_...` 当成
无效 JWT。

### 公共读取（无需令牌）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/hello` | 健康检查 |
| GET | `/api/posts` | 已发布文章分页；`{list,page,total}` |
| GET | `/api/posts/summary` | 首页卡片；`{list,page,total}` |
| GET | `/api/posts/dates` | 有文章的日期；裸数组 `[{date,count}]` |
| GET | `/api/posts/{slug}` | 已发布文章详情；默认浏览数 +1 |
| GET | `/api/search` | 搜索已发布文章；`keyword` 必填 |
| GET | `/api/categories` | 裸数组，含 `post_count` |
| GET | `/api/tags` | 裸数组，含 `post_count` |
| GET | `/api/pages` | 已发布页面；`{list,total}` |
| GET | `/api/pages/{slug}` | 已发布页面详情 |
| GET | `/api/comments` | 已通过评论；裸数组，`post_id` 可选 |

公共页面路由只认管理员 **JWT**。用 `sk_` 令牌访问 `/api/pages` 等同匿名，
看不到草稿；草稿用 `GET /api/admin/pages`。

## 3. 文章

### `GET /api/admin/posts/summary`

查询参数（全部可选）：

| 参数 | 说明 |
|---|---|
| `page` | 默认 1 |
| `page_size` | 默认 20 |
| `status` | `draft` / `published` / `hidden` |
| `category_id` | 分类 ID |
| `tag_id` | 标签 ID |
| `keyword` | 标题 `LIKE %keyword%` |
| `start_date` | `YYYY-MM-DD`，按 `DATE(created_at)` 比较，含当天 |
| `end_date` | `YYYY-MM-DD`，含当天 |

响应：

```json
{
  "list": [
    {
      "id": 42,
      "created_at": "2026-11-13T19:00:00+08:00",
      "title": "标题",
      "slug": "标题",
      "status": "draft",
      "view_count": 0
    }
  ],
  "page": 1,
  "page_size": 20,
  "total": 1
}
```

### `GET /api/admin/posts`

参数同上，返回 `PostDto` 完整对象（含 `content` / `html_content`）。列表只
预加载分类和标签，**`author` 是零值对象**（`id: 0`、空字符串）。

### `GET /api/admin/posts/_id/{id}` / `GET /api/admin/posts/slug/{slug}`

返回完整 `PostDto`，这两个详情端点会加载作者。

### `POST /api/admin/posts`

```json
{
  "title": "必填",
  "content": "必填，Markdown 原文",
  "slug": "可选；默认由标题生成",
  "excerpt": "可选",
  "cover_image": "可选，只接受已有 URL",
  "category_id": 1,
  "tag_ids": [1, 3],
  "status": "draft"
}
```

- `status` 省略或空字符串时后端默认 `draft`。
- 创建时 `slug` 重复返回 400 `{"error":"Slug already exists"}`。
- `tag_ids` 省略或 `[]` 表示不挂标签。
- 返回 201 `PostDto`；其中 `category`、`author` 是零值对象，`tags` 是刚
  保存的标签。创建后需要完整对象请再 `GET /api/admin/posts/_id/{id}`。

### `PUT /api/admin/posts/_id/{id}`

所有字段可选，但语义特殊：

- `title` / `slug` / `content` / `excerpt` / `cover_image` / `status` 传空字符串
  等于“保持不变”，不能用来清空。
- `category_id` 不传保持不变；传 `null` 也保持不变；无法清空分类。
- `tag_ids` 不传保持不变；传 `[]` 会加载并挂上**所有存活标签**（GORM
  `Find(&tags, []uint{})` 的已知怪癖）；传非空列表则整体替换。
- `slug` 冲突不会返回 400：保存错误被忽略，仍返回 200，数据库可能保留旧
  slug。更新后必须重新 GET 确认。
- 后端忽略保存错误，仍返回 200。
- 返回的是内存中的 `PostDto`：`category` / `author` 为零值；未传 `tag_ids` 时
  `tags` 为 `null`；时间戳可能不是更新后的值。以随后的 GET 为准。

## 4. 页面

### `GET /api/admin/pages`

参数：`page`（默认 1）、`page_size`（默认 20）。响应 `{list,total}`，每项是
`PageDto`（含 `content` / `html_content`）。

### `GET /api/admin/pages/_id/{id}`

返回 `PageDto`。没有“按 slug 读取后台页面”的端点；slug 查询只能走公共
`/api/pages/{slug}`（只返回 published），草稿需要先列后台页面再按 slug 匹配。

### `POST /api/admin/pages`

```json
{
  "title": "必填",
  "slug": "可选",
  "content": "可选，Markdown",
  "status": "draft"
}
```

后端在 `status` 为空时默认 `published`。`blog.py` 为了安全，创建时省略
`--status` 会显式发送 `draft`。要发布必须传 `--status published`。
创建时 slug 重复返回 500 `{"error":"Failed to create page"}`（不是 400）。

### `PUT /api/admin/pages/_id/{id}`

`title` / `slug` / `content` / `status` 传空字符串等于保持不变。保存错误被
忽略，仍返回 200；slug 冲突时数据库可能保留旧值。返回的是内存中的 `PageDto`，
更新后应重新读取确认。

## 5. 标签与分类

### 标签

- `GET /api/tags`：公共裸数组，每项 `{id,created_at,updated_at,name,slug,post_count}`。
- `POST /api/admin/tags`：`{"name":"必填","slug":"可选"}`，返回 201 `TagDto`；
  重复 slug 会以 500 `{"error":"Failed to create tag"}` 失败。
- `PUT /api/admin/tags/_id/{id}`：`name` / `slug` 可选，空字符串保持不变；
  slug 冲突返回 400 `{"error":"Slug already exists"}`。

### 分类

- `GET /api/categories`：公共裸数组，每项 `{id,created_at,updated_at,name,slug,description,sort,post_count}`。
- `POST /api/admin/categories`：`{"name":"必填","slug":"可选","description":"可选","sort":0}`；
  重复 slug 会以 500 `{"error":"Failed to create category"}` 失败。
- `PUT /api/admin/categories/_id/{id}`：`name` / `slug` / `description` 空字符串
  保持不变；`sort` 只在 `> 0` 时应用，传 0 无法把排序改回 0。slug 冲突的保存
  错误被忽略，仍返回 200，需重新读取确认。

## 6. 评论

### `GET /api/admin/comments`

参数：`page`（默认 1）、`page_size`（默认 20）、`status`
（`pending` / `approved` / `rejected`）。响应 `{list,total}`，每项是
`CommentDto`，其中 `post` 和 `user` 是完整对象。

### `POST /api/admin/comments/_id/{id}/approve` / `reject`

请求体 `{}` 即可。返回 `{"message":"Comment approved"}` 或
`{"message":"Comment rejected"}`。后端忽略保存错误，仍返回 200。

### `GET /api/comments?post_id={id}`

公共裸数组，只含 `approved` 顶层评论，附带一层 `children`（所有状态）。
没有令牌删除评论的能力；删除需要管理员 JWT/后台网页。

## 7. 响应形状

### PostDto（完整）

```json
{
  "id": 42,
  "created_at": "2026-11-13T19:00:00+08:00",
  "updated_at": "2026-11-13T19:00:00+08:00",
  "title": "标题",
  "slug": "标题",
  "content": "# Markdown",
  "html_content": "<h1>Markdown</h1>",
  "excerpt": "摘要",
  "cover_image": "https://...",
  "status": "published",
  "view_count": 3,
  "is_deleted": false,
  "category_id": 1,
  "category": {
    "id": 1,
    "created_at": "...",
    "updated_at": "...",
    "name": "分类",
    "slug": "分类",
    "description": "",
    "sort": 0
  },
  "tags": [
    {
      "id": 2,
      "created_at": "...",
      "updated_at": "...",
      "name": "标签",
      "slug": "标签"
    }
  ],
  "author_id": 1,
  "author": {
    "id": 1,
    "created_at": "...",
    "updated_at": "...",
    "email": "root@smileyan.cn",
    "nickname": "管理员",
    "avatar": "",
    "role": "admin",
    "status": "active"
  }
}
```

注意：

- `category` / `author` 在关联未加载或已软删除时是**零值对象**，不是 `null`。
- `tags` 在关联未加载时可能是 `null`，预加载到空结果时是 `[]`。

### PageDto

`{id,created_at,updated_at,title,slug,content,html_content,status}`。

### 列表信封

| 端点 | 形状 |
|---|---|
| `/api/posts`、`/api/posts/summary` | `{list,page,total}` |
| `/api/admin/posts`、`/api/admin/posts/summary` | `{list,page,page_size,total}` |
| `/api/admin/pages`、`/api/pages` | `{list,total}` |
| `/api/admin/comments` | `{list,total}` |
| `/api/search` | `{list,page,total}`，无命中时 `list` 为 `null` |
| `/api/categories`、`/api/tags`、`/api/comments`、`/api/posts/dates` | 裸数组 |

### 时间与枚举

- 时间：Go RFC3339 风格本地偏移，例如 `2026-11-13T19:00:00+08:00`，
  小数位 0–3 位；不是 epoch。
- 日期：`YYYY-MM-DD`。
- 文章状态：`draft` / `published` / `hidden`。
- 评论状态：`pending` / `approved` / `rejected`。
- 角色：`admin` / `reader`。

## 8. 分页与限制

- 公共文章列表默认 `page_size=10`；后台文章/页面/评论默认 `page_size=20`。
- 文章与评论的 `page_size` 走严格 `strconv.Atoi`：非法值变 0，`0` 返回空页；
  负数会取消 `LIMIT`（很重，CLI 只允许正整数）。
- 页面和订阅的页码解析会忽略非数字字符（`"1abc"` → 1），CLI 仍只传整数。
- JSON 请求体上限 2 MiB（axum 默认）。长文超过时需要拆分或改用其他方式。
- 公共 `GET /api/posts/{slug}` 每次会 `view_count + 1`；传 `?edit=true` 不增加。

## 9. 错误

统一为 `{"error":"..."}`，`Content-Type: application/json; charset=utf-8`。

| 状态 | 常见文案 | 原因 |
|---|---|---|
| 400 | `Invalid request` | 缺少标题/正文、字段类型错误 |
| 400 | `Slug already exists` | 文章创建或标签更新的 slug 冲突 |
| 400 | `Keyword is required` | 搜索没带 keyword |
| 401 | `Invalid or expired token` | 令牌无效/过期/被撤销，或对令牌用了 DELETE/管理员路由 |
| 401 | `Authorization header is required` | 完全没带认证头 |
| 403 | `Admin access required` | 带了有效 JWT 但不是 admin |
| 404 | `Post not found` / `Page not found` / `Comment not found` | 资源不存在或公共接口不可见 |
| 413 | 请求体过大 | 超过 2 MiB |
| 500 | `Failed to create post` 等 | 服务端错误 |

方法不匹配返回 `404 page not found`（不是 405）；尾斜杠重定向 GET 301、
其他方法 307。CLI 生成的路径没有尾斜杠。

## 10. curl 对照

```bash
BASE="${SMILEYAN_BLOG_URL:-http://127.0.0.1:8080}"

# 验证令牌
curl -sS -H "Authorization: Bearer $SMILEYAN_BLOG_TOKEN" \
  "$BASE/api/admin/posts/summary?page_size=1"

# 创建草稿
curl -sS -X POST \
  -H "Authorization: Bearer $SMILEYAN_BLOG_TOKEN" \
  -H "Content-Type: application/json" \
  --data '{"title":"标题","content":"# 正文","status":"draft"}' \
  "$BASE/api/admin/posts"

# 发布文章
curl -sS -X PUT \
  -H "Authorization: Bearer $SMILEYAN_BLOG_TOKEN" \
  -H "Content-Type: application/json" \
  --data '{"status":"published"}' \
  "$BASE/api/admin/posts/_id/42"
```
