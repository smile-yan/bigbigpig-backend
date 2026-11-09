-- Deterministic seed for contract / search capture.
--
-- All ids and timestamps are fixed so the captured fixtures are reproducible.
-- New rows created during capture start at id 100 (see AUTO_INCREMENT resets).
-- Timestamps are written in Asia/Shanghai (+08:00) because the Go baseline
-- runs with parseTime=True&loc=Local and the capture MySQL is pinned to +08:00.

SET NAMES utf8mb4;
SET time_zone = '+08:00';

-- ---------------------------------------------------------------------------
-- users
-- ---------------------------------------------------------------------------
INSERT INTO users (id, created_at, updated_at, deleted_at, email, nickname, avatar, role, status) VALUES
  (1, '2026-01-01 09:00:00.000', '2026-01-01 09:00:00.000', NULL, 'admin@example.com', 'Admin', '', 'admin', 'active'),
  (2, '2026-01-02 09:00:00.000', '2026-01-02 09:00:00.000', NULL, 'reader@example.com', 'Reader', '/uploads/avatars/2_seed.png', 'reader', 'active'),
  (3, '2026-01-03 09:00:00.000', '2026-01-03 09:00:00.000', '2026-03-01 00:00:00.000', 'ghost@example.com', 'Ghost', '', 'reader', 'active');

-- ---------------------------------------------------------------------------
-- categories
-- ---------------------------------------------------------------------------
INSERT INTO categories (id, created_at, updated_at, deleted_at, name, slug, description, sort) VALUES
  (1, '2026-01-01 09:00:00.000', '2026-01-01 09:00:00.000', NULL, '技术', 'tech', '技术类文章', 1),
  (2, '2026-01-01 09:00:00.000', '2026-01-01 09:00:00.000', NULL, '生活', 'life', '生活随笔', 2),
  (3, '2026-01-01 09:00:00.000', '2026-01-01 09:00:00.000', '2026-03-01 00:00:00.000', '归档', 'archived', '已删除分类', 3);

-- ---------------------------------------------------------------------------
-- tags
-- ---------------------------------------------------------------------------
INSERT INTO tags (id, created_at, updated_at, deleted_at, name, slug) VALUES
  (1, '2026-01-01 09:00:00.000', '2026-01-01 09:00:00.000', NULL, 'Go', 'go'),
  (2, '2026-01-01 09:00:00.000', '2026-01-01 09:00:00.000', NULL, 'Rust', 'rust'),
  (3, '2026-01-01 09:00:00.000', '2026-01-01 09:00:00.000', '2026-03-01 00:00:00.000', 'Legacy', 'legacy');

-- ---------------------------------------------------------------------------
-- posts (covers published / draft / hidden / is_deleted / deleted_at)
-- ---------------------------------------------------------------------------
INSERT INTO posts (id, created_at, updated_at, deleted_at, title, slug, content, html_content, excerpt, cover_image, status, view_count, is_deleted, category_id, author_id) VALUES
  (1, '2026-10-01 10:00:00.000', '2026-10-01 10:00:00.000', NULL,
   'Published Post 已发布', 'published-post',
   '# Published Post 已发布\n\n这是 Go 与 Rust 的契约测试文章，覆盖全文搜索、数据库与云原生。\n\n## 代码示例\n\n```go\nfmt.Println("hello")\n```\n',
   '<h1 class="article-h1">Published Post 已发布</h1>\n<p class="article-p">这是 Go 与 Rust 的契约测试文章，覆盖全文搜索、数据库与云原生。</p>',
   '', '', 'published', 10, 0, 1, 1),
  (2, '2026-10-05 10:00:00.000', '2026-10-05 10:00:00.000', NULL,
   'Draft Post 草稿', 'draft-post',
   '# Draft Post 草稿\n\n这篇草稿不应出现在公开搜索中。draft secret keyword zzqdraft。\n',
   '<h1 class="article-h1">Draft Post 草稿</h1>',
   '草稿摘要', '', 'draft', 0, 0, 1, 1),
  (3, '2026-10-06 10:00:00.000', '2026-10-06 10:00:00.000', NULL,
   'Hidden Post 隐藏', 'hidden-post',
   '# Hidden Post 隐藏\n\nhidden secret keyword zzqhidden。\n',
   '<h1 class="article-h1">Hidden Post 隐藏</h1>',
   '', '', 'hidden', 5, 0, 2, 2),
  (4, '2026-10-07 10:00:00.000', '2026-10-07 10:00:00.000', NULL,
   'Deleted Post 已删除', 'deletedpost',
   '# Deleted Post 已删除\n\ndeleted secret keyword zzqdeleted。\n',
   '<h1 class="article-h1">Deleted Post 已删除</h1>',
   '', '', 'hidden', 7, 1, 1, 1),
  (5, '2026-10-08 10:00:00.000', '2026-10-08 10:00:00.000', '2026-10-09 10:00:00.000',
   'Soft Deleted At Post', 'soft-deleted-at-post',
   '# Soft Deleted At Post\n\nsoftdeleted zzqsoft。\n',
   '<h1 class="article-h1">Soft Deleted At Post</h1>',
   '', '', 'published', 3, 0, 1, 1),
  (6, '2026-10-03 10:00:00.000', '2026-10-03 10:00:00.000', NULL,
   '无分类文章', 'published-no-category',
   '# 无分类文章\n\nGo MySQL Redis Bleve 全文搜索。\n',
   '<h1 class="article-h1">无分类文章</h1>',
   '固定摘要', 'https://example.com/cover.png', 'published', 11, 0, NULL, 2),
  (7, '2026-10-04 10:00:00.000', '2026-10-04 10:00:00.000', NULL,
   '长文摘要边界', 'published-long',
   CONCAT('# 长文摘要边界\n\n', REPEAT('长文摘要测试', 40), '\n'),
   '<h1 class="article-h1">长文摘要边界</h1>',
   '', '', 'published', 2, 0, 1, 1),
  (8, '2026-09-15 10:00:00.000', '2026-09-15 10:00:00.000', NULL,
   'September Archive', 'published-september',
   '# September Archive\n\nSeptember archive post about Linux, Docker, Nginx.\n',
   '<h1 class="article-h1">September Archive</h1>',
   '', '', 'published', 4, 0, 2, 1),
  (9, '2026-10-09 10:00:00.000', '2026-10-09 10:00:00.000', NULL,
   'Published But IsDeleted', 'published-but-is-deleted',
   '# Published But IsDeleted\n\npublished-is-deleted leak keyword zzqpubdel。\n',
   '<h1 class="article-h1">Published But IsDeleted</h1>',
   '', '', 'published', 8, 1, 1, 1);

-- ---------------------------------------------------------------------------
-- post_tags
-- post 6 references a soft-deleted tag (tag 3) on purpose.
-- ---------------------------------------------------------------------------
INSERT INTO post_tags (post_id, tag_id) VALUES
  (1, 1), (1, 2),
  (2, 1),
  (3, 2),
  (4, 1),
  (5, 2),
  (6, 3),
  (7, 1),
  (8, 2),
  (9, 1);

-- ---------------------------------------------------------------------------
-- pages (published / draft / deleted_at)
-- ---------------------------------------------------------------------------
INSERT INTO pages (id, created_at, updated_at, deleted_at, title, slug, content, html_content, status) VALUES
  (1, '2026-01-10 09:00:00.000', '2026-01-10 09:00:00.000', NULL,
   'About', 'about', '# About\n\n关于页面。\n', '<h1 class="article-h1">About</h1>', 'published'),
  (2, '2026-01-11 09:00:00.000', '2026-01-11 09:00:00.000', NULL,
   'Draft Page', 'draft-page', '# Draft Page\n\n草稿页面。\n', '<h1 class="article-h1">Draft Page</h1>', 'draft'),
  (3, '2026-01-12 09:00:00.000', '2026-01-12 09:00:00.000', '2026-03-01 00:00:00.000',
   'Legacy Page', 'legacy-page', '# Legacy Page\n\n已删除页面。\n', '<h1 class="article-h1">Legacy Page</h1>', 'published');

-- ---------------------------------------------------------------------------
-- comments
-- comment 3 is a reply to comment 2; comment 8 replies to comment 3.
-- comment 5 is soft-deleted; comment 6 is rejected; comment 7 belongs to a draft post.
-- ---------------------------------------------------------------------------
INSERT INTO comments (id, created_at, updated_at, deleted_at, content, status, post_id, user_id, parent_id) VALUES
  (1, '2026-10-02 09:00:00.000', '2026-10-02 09:00:00.000', NULL, '读者评论 approved', 'approved', 1, 2, NULL),
  (2, '2026-10-02 10:00:00.000', '2026-10-02 10:00:00.000', NULL, '管理员评论 approved', 'approved', 1, 1, NULL),
  (3, '2026-10-02 11:00:00.000', '2026-10-02 11:00:00.000', NULL, '对管理员评论的回复 approved', 'approved', 1, 2, 2),
  (4, '2026-10-02 12:00:00.000', '2026-10-02 12:00:00.000', NULL, '待审核评论 pending', 'pending', 1, 2, NULL),
  (5, '2026-10-02 13:00:00.000', '2026-10-02 13:00:00.000', '2026-10-03 00:00:00.000', '已删除评论 deleted', 'approved', 1, 2, NULL),
  (6, '2026-10-02 14:00:00.000', '2026-10-02 14:00:00.000', NULL, '已拒绝评论 rejected', 'rejected', 1, 2, NULL),
  (7, '2026-10-02 15:00:00.000', '2026-10-02 15:00:00.000', NULL, '草稿文章下的评论 approved', 'approved', 2, 2, NULL),
  (8, '2026-10-02 16:00:00.000', '2026-10-02 16:00:00.000', NULL, '对回复的二级回复 approved', 'approved', 1, 1, 3);

-- ---------------------------------------------------------------------------
-- subscriptions
-- a@example.com is active, b@ inactive, c@ soft-deleted,
-- deleted-sub@ is soft-deleted and used to capture the Q19 duplicate-key path.
-- ---------------------------------------------------------------------------
INSERT INTO subscriptions (id, created_at, updated_at, deleted_at, email, token, is_active) VALUES
  (1, '2026-01-20 09:00:00.000', '2026-01-20 09:00:00.000', NULL, 'a@example.com', '0123456789abcdef0123456789abcdef', 1),
  (2, '2026-01-21 09:00:00.000', '2026-01-21 09:00:00.000', NULL, 'b@example.com', 'fedcba9876543210fedcba9876543210', 0),
  (3, '2026-01-22 09:00:00.000', '2026-01-22 09:00:00.000', '2026-03-01 00:00:00.000', 'c@example.com', '11111111111111111111111111111111', 1),
  (4, '2026-01-23 09:00:00.000', '2026-01-23 09:00:00.000', '2026-03-01 00:00:00.000', 'deleted-sub@example.com', '22222222222222222222222222222222', 1);

-- New rows created during capture must be deterministic across runs.
ALTER TABLE users AUTO_INCREMENT = 100;
ALTER TABLE categories AUTO_INCREMENT = 100;
ALTER TABLE tags AUTO_INCREMENT = 100;
ALTER TABLE posts AUTO_INCREMENT = 100;
ALTER TABLE pages AUTO_INCREMENT = 100;
ALTER TABLE comments AUTO_INCREMENT = 100;
ALTER TABLE subscriptions AUTO_INCREMENT = 100;
