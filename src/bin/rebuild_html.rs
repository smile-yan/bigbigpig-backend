//! Re-render `html_content` for every post and page.
//!
//! Equivalent to `smileyan-backend/scripts/rebuild_html.go`: load the config,
//! connect to MySQL, re-run the Markdown pipeline and update only rows whose
//! HTML changed. Run this after changing `services/markdown.rs`.
//!
//! ```text
//! cargo run --bin rebuild_html
//! ```

use anyhow::Context;
use sqlx::MySqlPool;

use smileyan_backend::config;
use smileyan_backend::services::markdown;
use smileyan_backend::state;

#[derive(Debug, sqlx::FromRow)]
struct ContentRow {
    id: u64,
    content: Option<String>,
    html_content: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = config::load()?;
    let pool = state::connect_db(&cfg).await?;

    println!("开始重新渲染文章 HTML...");

    let posts: Vec<ContentRow> = sqlx::query_as(
        "SELECT id, content, html_content FROM posts WHERE deleted_at IS NULL ORDER BY id ASC",
    )
    .fetch_all(&pool)
    .await
    .context("查询文章失败")?;

    let updated_posts = rebuild(
        &pool,
        &posts,
        "UPDATE posts SET html_content = ?, updated_at = NOW(3) WHERE id = ?",
    )
    .await?;
    println!("文章: 共 {} 篇，更新 {} 篇", posts.len(), updated_posts);

    let pages: Vec<ContentRow> = sqlx::query_as(
        "SELECT id, content, html_content FROM pages WHERE deleted_at IS NULL ORDER BY id ASC",
    )
    .fetch_all(&pool)
    .await
    .context("查询页面失败")?;

    let updated_pages = rebuild(
        &pool,
        &pages,
        "UPDATE pages SET html_content = ?, updated_at = NOW(3) WHERE id = ?",
    )
    .await?;
    println!("页面: 共 {} 个，更新 {} 个", pages.len(), updated_pages);

    println!("处理完成！");
    Ok(())
}

async fn rebuild(pool: &MySqlPool, rows: &[ContentRow], statement: &str) -> anyhow::Result<usize> {
    let mut updated = 0;
    for row in rows {
        let content = row.content.as_deref().unwrap_or_default();
        let new_html = markdown::render(content);
        if new_html == row.html_content.as_deref().unwrap_or_default() {
            continue;
        }

        if let Err(err) = sqlx::query(sqlx::AssertSqlSafe(statement))
            .bind(&new_html)
            .bind(row.id)
            .execute(pool)
            .await
        {
            eprintln!("更新 [{}] 失败: {err}", row.id);
        } else {
            updated += 1;
        }
    }
    Ok(updated)
}
