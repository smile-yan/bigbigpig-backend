//! `GET /api/search`.
//!
//! The response is assembled from the in-memory index (slug order and `total`)
//! and MySQL (the complete post shape), exactly like `controllers/search.go`.
//! Q1/Q2 fixes live in `services::search`; this module only applies the final
//! published/not-deleted guard before returning a row.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::error::AppError;
use crate::models::go_time;
use crate::models::{
    POST_STATUS_PUBLISHED, SearchCategoryDto, SearchResponse, SearchResultDto, SearchTagDto,
};
use crate::repo::posts;
use crate::services::params::parse_gin_int;
use crate::services::search::excerpt_from_content;
use crate::state::SharedState;

const EXCERPT_BYTES: usize = 200;

#[derive(Debug, Default, Deserialize)]
pub struct SearchQuery {
    keyword: Option<String>,
    page: Option<String>,
    page_size: Option<String>,
}

pub async fn search(
    State(state): State<SharedState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, AppError> {
    let keyword = query.keyword.unwrap_or_default();
    if keyword.is_empty() {
        return Err(AppError::BadRequest("Keyword is required"));
    }

    // `strconv.Atoi`: invalid values become 0, and the response echoes the
    // parsed value (the Go handler includes `page`, not `page_size`).
    let page = query.page.as_deref().map(parse_gin_int).unwrap_or(1);
    let page_size = query.page_size.as_deref().map(parse_gin_int).unwrap_or(10);
    let (limit, offset) = search_pagination(page, page_size);

    let hits = state
        .search
        .search(&keyword, page, page_size)
        .map_err(|err| AppError::internal_msg("Search failed", err))?;

    let mut total = hits.total as i64;
    let (rows, ordered_slugs) = if hits.slugs.is_empty() && total == 0 {
        // `fallback_like`: the index found nothing. Look for a literal
        // substring in MySQL so tokenizer regressions cannot blank the page.
        if state.cfg.search.fallback_like {
            let count = posts::count_like(&state.db, &keyword)
                .await
                .map_err(|err| AppError::internal_msg("Search failed", err))?;
            total = count;
            if count == 0 || limit <= 0 {
                (Vec::new(), Vec::new())
            } else {
                let rows = posts::list_like(&state.db, &keyword, limit, offset)
                    .await
                    .map_err(|err| AppError::internal_msg("Search failed", err))?;
                let slugs = rows.iter().map(|row| row.slug.clone()).collect();
                (rows, slugs)
            }
        } else {
            (Vec::new(), Vec::new())
        }
    } else {
        let rows = posts::find_by_slugs(&state.db, &hits.slugs)
            .await
            .map_err(|err| AppError::internal_msg("Search failed", err))?;
        (rows, hits.slugs)
    };

    let list = assemble_results(&state, rows, &ordered_slugs).await?;

    Ok(Json(SearchResponse {
        list: if list.is_empty() { None } else { Some(list) },
        page,
        total,
    }))
}

/// Load category/tag relations for the hit rows, restore the score order and
/// drop rows that are no longer published (Q2 guard for a stale index).
async fn assemble_results(
    state: &SharedState,
    rows: Vec<crate::models::PostRow>,
    ordered_slugs: &[String],
) -> Result<Vec<SearchResultDto>, AppError> {
    let mut by_slug: HashMap<String, crate::models::PostRow> = rows
        .into_iter()
        .filter(|row| {
            row.status.as_deref() == Some(POST_STATUS_PUBLISHED) && !row.is_deleted.unwrap_or(false)
        })
        .map(|row| (row.slug.clone(), row))
        .collect();

    let kept: Vec<crate::models::PostRow> = ordered_slugs
        .iter()
        .filter_map(|slug| by_slug.remove(slug))
        .collect();
    if kept.is_empty() {
        return Ok(Vec::new());
    }

    let category_ids: Vec<u64> = kept
        .iter()
        .filter_map(|row| row.category_id)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let post_ids: Vec<u64> = kept.iter().map(|row| row.id).collect();

    let categories = posts::categories_for_posts(&state.db, &category_ids)
        .await
        .map_err(|err| AppError::internal_msg("Search failed", err))?;
    let categories: HashMap<u64, crate::models::CategoryRow> =
        categories.into_iter().map(|row| (row.id, row)).collect();
    let tag_rows = posts::tags_for_posts(&state.db, &post_ids)
        .await
        .map_err(|err| AppError::internal_msg("Search failed", err))?;

    let mut tags: HashMap<u64, Vec<SearchTagDto>> = HashMap::new();
    for tag in tag_rows {
        tags.entry(tag.post_id).or_default().push(SearchTagDto {
            id: tag.id,
            name: tag.name,
        });
    }

    Ok(kept
        .into_iter()
        .map(|row| {
            let excerpt = row.excerpt.clone().unwrap_or_default();
            let excerpt = if excerpt.is_empty() {
                row.content
                    .as_deref()
                    .filter(|content| !content.is_empty())
                    .map(|content| excerpt_from_content(content, EXCERPT_BYTES))
                    .unwrap_or_default()
            } else {
                excerpt
            };

            SearchResultDto {
                category: row.category_id.map(|id| {
                    let category = categories.get(&id);
                    SearchCategoryDto {
                        id: category.map(|category| category.id).unwrap_or(0),
                        name: category
                            .map(|category| category.name.clone())
                            .unwrap_or_default(),
                    }
                }),
                cover_image: row.cover_image.unwrap_or_default(),
                created_at: go_time::format_opt(row.created_at, state.tz),
                excerpt,
                id: row.id,
                slug: row.slug,
                status: row.status.unwrap_or_default(),
                tags: tags.remove(&row.id).filter(|tags| !tags.is_empty()),
                title: row.title,
                view_count: row.view_count.unwrap_or(0),
            }
        })
        .collect())
}

/// Go's search pagination: `From=(page-1)*page_size`, `Size=page_size`;
/// non-positive sizes return no rows.
fn search_pagination(page: i64, page_size: i64) -> (i64, i64) {
    if page_size <= 0 {
        return (0, 0);
    }
    let offset = page.saturating_sub(1).max(0).saturating_mul(page_size);
    (page_size, offset)
}
