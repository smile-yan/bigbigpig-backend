---
name: smileyan-blog
description: 管理 Smileyan 博客（smileyan.cn）。当用户要发布、创建、修改、查看文章或页面，管理标签/分类，审核评论，搜索博客内容，处理草稿/发布状态，或提到 Smileyan 博客、SMILEYAN_BLOG_TOKEN、sk_ 令牌时使用。通过后台「令牌管理」创建的 API 令牌调用 REST API；不处理删除、用户、订阅、通知、TTS。
when_to_use: 用户提到 Smileyan 博客、smileyan.cn、发布或修改博客文章、创建页面、管理标签/分类、审核评论、搜索文章、查看草稿，或需要用到 SMILEYAN_BLOG_TOKEN / sk_ 令牌时。
---

# Smileyan 博客管理

通过管理员在博客后台「令牌管理」创建的 API 令牌（`sk_...`）管理内容。所有操作
走本 skill 目录下的 `scripts/blog.py`；它只依赖 Python 3 标准库，统一处理认证、
JSON、分页和错误。完整端点与字段见 `references/api.md`。

## 1. 准备与验证

1. 令牌来自 `SMILEYAN_BLOG_TOKEN`，必须是 `sk_` 开头；本 skill 不接受管理员 JWT。
2. 地址来自 `SMILEYAN_BLOG_URL`，默认 `http://127.0.0.1:8080`。要管理线上博客，
   先确认该地址确实暴露了带令牌功能的 API，再用环境变量覆盖；不要假设
   `https://smileyan.cn/api` 一定可用（当前线上 `/api` 由 Caddy 返回 404）。
3. 当前 shell 没有继承变量时，CLI 会只读扫描 `~/.zshenv`、`~/.zprofile`、
   `~/.zshrc`、`~/.bash_profile`、`~/.bashrc`、`~/.profile` 里的同名 `export`
   行作为兜底（只解析，不执行文件，不打印值）。更稳妥的做法是把两行放到
   `~/.zshenv`，这样所有 zsh（包括非交互 shell）都会加载。
4. 先运行 `python3 <本 skill 目录>/scripts/blog.py check`。返回
   `{"ok": true, ...}` 才继续。`check` 返回 404 说明该地址不是当前后端或没有
   代理 `/api`，不要继续写操作。

如果缺少令牌或返回 401，让用户在前端后台「令牌管理」创建/续期令牌，并在启动
ZCode 的环境（shell profile 或启动配置）里 `export SMILEYAN_BLOG_TOKEN=sk_...`；
只在另一个终端临时 export 不一定能被 agent 的 shell 继承。不要把令牌写进命令、
文件、日志或回复；如果用户直接在聊天里贴了令牌，不要复述它。

## 2. 能力边界

令牌可以：

- 文章：后台列表/详情、创建、更新（含发布/转草稿）
- 页面：后台列表/详情、创建、更新
- 标签/分类：公共列表、创建、更新
- 评论：后台列表、通过/拒绝审核
- 公共读取：搜索、已发布文章/页面/评论/分类/标签、发布日历

令牌**不能**：删除任何内容（文章、页面、评论、标签、分类），以及用户管理、
订阅、通知、文章恢复、头像上传、TTS、令牌管理。遇到删除请求，说明需要管理员
JWT 或博客后台网页操作；不要尝试 `api DELETE`（CLI 会拒绝），也不要伪造 JWT。

## 3. 发布或更新文章

1. **定位文章**：`blog.py posts list --keyword 关键词 --page-size 50`，或
   `blog.py posts get <id|slug>`。列表默认返回轻量行；需要正文时加 `--full`。
2. **准备正文**：用 Write 工具把 Markdown 写到临时文件（例如
   `/tmp/smileyan-post.md`），再用 `--content-file` 传入。正文不能为空。
3. **新建**：
   ```bash
   python3 <skill>/scripts/blog.py posts create \
     --title "标题" --content-file /tmp/smileyan-post.md \
     --category-id 1 --tag-ids 2,5
   ```
   省略 `--status` 时是草稿。只有用户明确要求“发布/上线”时才加
   `--status published`。
4. **更新**：只传要改的字段：
   ```bash
   python3 <skill>/scripts/blog.py posts update 42 \
     --title "新标题" --status published
   ```
5. **验证**：更新后重新 `posts get <id>`，确认 `status`、`slug`、`tags`、
   `category_id` 符合预期，再向用户汇报。

关键规则：

- 更新时传空字符串等于“保持原值”，无法清空 `excerpt`、`cover_image` 或
  `category_id`。
- 更新标签必须传**完整的目标列表**；省略 `--tag-ids` 保持原标签。传空列表会被
  CLI 拒绝，因为后端会把 `tag_ids: []` 解释成“挂上所有标签”。
- 更新响应里的 `category` / `author` 是零值对象，未传 `--tag-ids` 时 `tags`
  可能是 `null`，时间戳也可能偏旧；以重新 GET 的结果为准。
- 不传 `--slug` 时由标题生成：保留中文，ASCII 转小写，空格变 `-`，标点被丢弃。
  创建时 slug 重复返回 400 `Slug already exists`；更新时 slug 冲突不会报错
  （保存错误被忽略，仍返回 200），务必重新 GET 确认 slug 是否真的改了。
- 后端 JSON 请求体上限 2 MiB；超长文章需要拆分或改用其他方式。

## 4. 页面

- 列表：`blog.py pages list`（含草稿和正文；没有轻量版）。
- 读取：`blog.py pages get <id>`；`pages get <slug>` 会先走公共接口，再回退到
  后台列表匹配，所以草稿也能读到。
- 新建：
  ```bash
  python3 <skill>/scripts/blog.py pages create \
    --title "关于" --content-file /tmp/about.md --status draft
  ```
  CLI 默认存为 `draft`（比后端默认的 `published` 更保守）。用户明确要上线时
  才传 `--status published`。
- 更新：`pages update <id> --title ... --content-file ... --status ...`；空字符串
  同样表示保持不变。

页面没有分类、标签、封面字段；正文是 Markdown。

## 5. 标签与分类

- 先读列表拿 ID 和现有 slug：
  `blog.py tags list`、`blog.py categories list`。
- 新建：
  ```bash
  python3 <skill>/scripts/blog.py tags create --name "Rust"
  python3 <skill>/scripts/blog.py categories create --name "技术" --sort 10
  ```
- 更新：
  ```bash
  python3 <skill>/scripts/blog.py tags update 9 --name "Rust 语言"
  python3 <skill>/scripts/blog.py categories update 2 --sort 20
  ```
- 标签**更新**时 slug 冲突返回 400 `Slug already exists`；标签/分类**创建**时
  重复 slug 会以 500（`Failed to create tag/category`）失败；分类**更新**的冲突
  保存错误被忽略，仍返回 200，必须重新读取确认。
- 分类的 `sort` 只在 `> 0` 时生效，传 0 无法把排序改回 0。
- 令牌不能删除标签/分类；需要删除时让用户在后台操作。

## 6. 评论审核

```bash
# 待审核列表
python3 <skill>/scripts/blog.py comments list --status pending

# 通过 / 拒绝
python3 <skill>/scripts/blog.py comments approve 7
python3 <skill>/scripts/blog.py comments reject 8
```

- 后台列表返回 `{list,total}`，含评论正文、状态、文章标题和用户信息。
- 公共评论（只看已通过）：`comments public --post-id 42`。
- 令牌不能删除评论；删除需要管理员 JWT/后台网页。
- 汇报时只列必要的评论内容和操作结果，不要无必要地输出用户邮箱。

## 7. 搜索与阅读

```bash
python3 <skill>/scripts/blog.py search "关键词" --page-size 10
python3 <skill>/scripts/blog.py read <slug> --no-view-count
python3 <skill>/scripts/blog.py dates
```

- 搜索只命中已发布、未删除文章；无命中时响应的 `list` 是 `null`，按“没有结果”
  处理。相关性顺序与旧 Bleve 不同，但命中集合和 `total` 一致。
- `read` 走公共详情；默认会 `view_count + 1`，只是查看内容时加
  `--no-view-count`。草稿用 `posts get`。
- `dates` 返回 `[{date,count}]` 裸数组，用于日历。

## 8. 分页与日期

- 公共文章列表默认 `page_size=10`；后台文章、页面、评论默认 20。
- CLI 只接受正整数；要扫描更多行就显式传 `--page-size 100`（必要时翻页）。
- 日期筛选 `--start-date/--end-date` 用 `YYYY-MM-DD`，按 `DATE(created_at)`
  比较且包含当天。
- 列表信封：公共文章 `{list,page,total}`；后台文章
  `{list,page,page_size,total}`；页面/评论 `{list,total}`；分类/标签/评论公共
  列表是裸数组。

## 9. 错误处理

`blog.py` 成功时把 JSON 打到 stdout，失败时把 `HTTP <status>: <body>` 和提示打到
stderr，并以非零码退出。

- **401**：令牌无效、过期或被撤销。让用户去后台「令牌管理」续期/重建，更新
  `SMILEYAN_BLOG_TOKEN` 后重试；不要反复重试同一个失效令牌。
- **403**：该操作需要管理员 JWT，API 令牌没有权限。
- **400**：字段不合法；常见原因是标题/正文为空、文章创建/标签更新时 slug
  重复、状态值不支持。
- **404**：资源不存在，或公共接口只返回已发布内容；草稿用后台 `posts get` /
  `pages get`。
- **413**：请求体超过 2 MiB。
- **网络错误**：检查 `SMILEYAN_BLOG_URL` 是否可达；本地实例默认
  `http://127.0.0.1:8080`。

## 10. 汇报格式

写操作完成后，用简短中文汇报：操作类型、资源 ID、标题、slug、当前状态、验证
结果；发布时附上公开链接：

- 文章：`https://smileyan.cn/post/<slug>`
- 页面：`https://smileyan.cn/page/<slug>`

不要粘贴完整 HTML 或整篇正文，除非用户要求。需要端点细节、请求/响应字段或
后端怪癖时，读 `references/api.md`。
