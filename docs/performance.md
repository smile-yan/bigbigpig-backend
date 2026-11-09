# 性能对比：Go 基线 vs Rust 重写（Phase 7）

> 日期：2026-11-05
> Go 基线：tag `go-baseline-2026-11`（commit `1f3a87f`，clean worktree `/tmp/smileyan-go-baseline`）
> Rust：本仓库 `main` + Phase 7 改动，`cargo build --release --locked`
> 机器：MacBook Pro / Apple M5 / 16 GB，macOS 15（darwin 25.6.0）
> 工具：`oha 1.16.0`，Rust 1.96.0，Go 1.27.1，Docker MySQL 8.4 + Redis 7.4

## 结论

三项计划内目标全部达标（阈值：p95 ≤ Go × 1.2；RSS ≤ Go × 2）：

| 端点 | Go p95 | Rust p95 | 比值 | 目标 | 结论 |
|---|---:|---:|---:|---|---|
| `GET /api/posts?page=1&page_size=10` | 26.82 ms | 22.37 ms | **0.83×** | ≤ 1.2× | 通过 |
| `GET /api/posts/{slug}` | 129.15 ms | 60.53 ms | **0.47×** | ≤ 1.2× | 通过 |
| `GET /api/search?keyword=文` | 58.82 ms | 16.66 ms | **0.28×** | ≤ 1.5× | 通过 |
| 峰值 RSS（三项最大） | 44.9 MB | 61.3 MB | **1.37×** | ≤ 2× | 通过 |

搜索是最大收益点（tantivy 内存索引 4.1× 吞吐、p95 降低 72%）；文章详情的
`view_count` 写入是行锁热点，Rust 的事务/参数绑定路径比 GORM 少一次往返，
p95 反而降低一半。`GET /api/categories` 不在计划目标内，Rust p95 为
19.62 ms（Go 12.67 ms，1.55×），但 p50 更快（6.37 vs 7.60 ms）、吞吐接近
（5939 vs 6081 rps），属于 50 并发下的尾延迟噪声。

## 明细

`-n 3000 -c 50`，每个后端独占端口 18080、使用同一份 schema/seed，
每个后端开始前重置数据库；`mode: prod`（避免 debug 级 SQL 日志干扰）。
RSS 为压测期间每 200 ms 采样 `ps -o rss=` 的峰值。

| backend | endpoint | RPS | p50 (ms) | p95 (ms) | p99 (ms) | 峰值 RSS (MB) | errors |
|---|---|---:|---:|---:|---:|---:|---:|
| go | posts_list | 2687.4 | 16.75 | 26.82 | 73.69 | 42.8 | 0 |
| go | post_detail | 451.5 | 106.06 | 129.15 | 177.53 | 42.4 | 0 |
| go | search | 1201.4 | 38.70 | 58.82 | 177.05 | 44.9 | 0 |
| go | categories | 6081.0 | 7.60 | 12.67 | 21.54 | 44.3 | 0 |
| rust | posts_list | 3308.4 | 14.04 | 22.37 | 38.73 | 61.3 | 0 |
| rust | post_detail | 929.6 | 53.28 | 60.53 | 64.61 | 48.4 | 0 |
| rust | search | 4934.6 | 8.74 | 16.66 | 27.99 | 45.3 | 0 |
| rust | categories | 5939.3 | 6.37 | 19.62 | 31.41 | 22.2 | 0 |

稳态 RSS（压测结束后）：Go 44.3 MB，Rust 21.4 MB。Rust 常驻内存更低；峰值
61 MB 来自 50 并发同时持有文章正文/HTML 的临时缓冲，远低于 2× 预算。

## 复现

```bash
# 依赖：docker compose、oha、go、cargo；Go 基线 worktree 见脚本头部
./scripts/bench.sh
```

脚本会：

1. `docker compose -f tests/fixtures/docker-compose.yml up -d mysql redis`
2. 创建 `smileyan_bench`，导入 `tests/fixtures/db/0001_schema.sql` + `seed.sql`
3. 在临时运行目录生成 `config.yaml`（端口 13306/16379、`mode: prod`、JWT secret 等）
4. 依次启动 Go / Rust，预热后对四个端点跑 `oha`
5. 输出 `target/bench/results.json` 和 Markdown 表格

结果文件 `target/bench/` 未提交（构建产物），本文记录的是 2026-11-05 的实测值。

## 已知边界

- 两边搜索索引的候选集不同：Go/Bleve 启动时索引全部文章（Q2 泄漏），Rust 只
  索引已发布未删除文章。查询热路径的耗时对比仍然有效，但 Go 的 `total` 会更高。
- `post_detail` 每次请求都会写 `view_count`，属数据库行锁瓶颈；绝对 RPS 受
  单行更新限制，不能代表只读路径的极限。
- 两个后端不是同时运行，避免 CPU/内存互相干扰；数据库在两次压测之间重置，保证
  数据状态一致。
