//! Post endpoints: public lists/details and the admin CRUD/restore surface.
//!
//! Behavioral notes carried over from `controllers/post.go`:
//! - Q2b (fix): public list/detail exclude `is_deleted` rows.
//! - Q3 (keep): the public routes have no authentication at all, so `edit=true`
//!   only suppresses the view counter; it does not reveal drafts.
//! - Q5 (keep): soft delete randomizes the slug and restore does not bring it
//!   back; a second DELETE soft-deletes again, only a DELETE on a `hidden`
//!   post hard-deletes.
//! - Q6 (fix): excerpts are cut on a UTF-8 boundary.
//! - Q25 (keep): the view counter update also refreshes `updated_at`.

use std::collections::{BTreeSet, HashMap};

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use serde::Deserialize;

use crate::error::AppError;
use crate::middleware::auth::{CurrentUser, resolve_current_user};
use crate::models::{
    AdminPostListResponse, AdminPostSummaryDto, AdminPostSummaryListResponse, CategoryDto,
    CategoryRow, MessageResponse, POST_STATUS_DRAFT, POST_STATUS_HIDDEN, POST_STATUS_PUBLISHED,
    PostDateCountDto, PostDto, PostRow, PostSummaryDto, PostSummaryTagDto, PostTagRow,
    PublicPostListResponse, PublicPostSummaryListResponse, ROLE_ADMIN, TagDto, UserDto, UserRow,
};
use crate::repo::posts::{self, PostFilter};
use crate::services::markdown;
use crate::services::params::parse_gin_int;
use crate::services::search::{self, IndexedPost};
use crate::services::slug;
use crate::state::SharedState;

const EXCERPT_BYTES: usize = 200;

#[derive(Debug, Default, Deserialize)]
pub struct PostListQuery {
    page: Option<String>,
    page_size: Option<String>,
    category_id: Option<String>,
    tag_id: Option<String>,
    keyword: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    status: Option<String>,
}

/// `GET /api/posts`
pub async fn get_posts(
    State(state): State<SharedState>,
    Query(query): Query<PostListQuery>,
) -> Result<Json<PublicPostListResponse>, AppError> {
    let page = query.page.as_deref().map(parse_gin_int).unwrap_or(1);
    let page_size = query.page_size.as_deref().map(parse_gin_int).unwrap_or(10);

    let filter = PostFilter {
        published_only: true,
        hide_deleted: true,
        category_id: non_empty(query.category_id),
        tag_id: non_empty(query.tag_id),
        keyword: non_empty(query.keyword),
        start_date: non_empty(query.start_date),
        end_date: non_empty(query.end_date),
        ..PostFilter::default()
    };

    let (limit, offset) = pagination(page, page_size);
    let total = posts::count(&state.db, &filter).await.map_err(internal)?;
    let rows = posts::list(&state.db, &filter, limit, offset)
        .await
        .map_err(internal)?;

    let relations = load_relations(&state, &rows, true)
        .await
        .map_err(internal)?;
    let list = rows
        .into_iter()
        .map(|row| {
            let mut dto = assemble_post(&state, row, &relations);
            if dto.excerpt.is_empty() && !dto.content.is_empty() {
                dto.excerpt = markdown::excerpt(&dto.content, EXCERPT_BYTES);
            }
            dto
        })
        .collect();

    Ok(Json(PublicPostListResponse { list, page, total }))
}

/// `GET /api/posts/summary` — the home-page post list.
///
/// Additive endpoint (no Go baseline equivalent): the same filters and
/// pagination as `GET /api/posts`, but it selects only the columns a card
/// renders, joins the category name instead of loading categories separately,
/// skips the author, and runs the count and page queries concurrently. The
/// article body (`content`/`html_content`) never leaves MySQL.
pub async fn get_posts_summary(
    State(state): State<SharedState>,
    Query(query): Query<PostListQuery>,
) -> Result<Json<PublicPostSummaryListResponse>, AppError> {
    let page = query.page.as_deref().map(parse_gin_int).unwrap_or(1);
    let page_size = query.page_size.as_deref().map(parse_gin_int).unwrap_or(10);

    let filter = PostFilter {
        published_only: true,
        hide_deleted: true,
        category_id: non_empty(query.category_id),
        tag_id: non_empty(query.tag_id),
        keyword: non_empty(query.keyword),
        start_date: non_empty(query.start_date),
        end_date: non_empty(query.end_date),
        ..PostFilter::default()
    };

    let (limit, offset) = pagination(page, page_size);
    // The two statements are independent; issuing them together saves one
    // round trip to the database on every request.
    let (total, rows) = tokio::try_join!(
        posts::count(&state.db, &filter),
        posts::list_summary(&state.db, &filter, limit, offset),
    )
    .map_err(internal)?;

    let post_ids: Vec<u64> = rows.iter().map(|row| row.id).collect();
    let tag_rows = posts::summary_tags_for_posts(&state.db, &post_ids)
        .await
        .map_err(internal)?;
    let mut tags: HashMap<u64, Vec<PostSummaryTagDto>> = HashMap::new();
    for tag in tag_rows {
        tags.entry(tag.post_id)
            .or_default()
            .push(PostSummaryTagDto {
                id: tag.id,
                name: tag.name,
            });
    }

    // The public list derives an excerpt from the body when the stored one is
    // empty. Read prefixes for those rows only, so the usual page stays free
    // of `content`.
    let needs_excerpt: Vec<u64> = rows
        .iter()
        .filter(|row| row.excerpt.as_deref().unwrap_or_default().is_empty())
        .map(|row| row.id)
        .collect();
    let mut previews: HashMap<u64, String> = HashMap::new();
    if !needs_excerpt.is_empty() {
        for row in posts::content_previews(&state.db, &needs_excerpt)
            .await
            .map_err(internal)?
        {
            previews.insert(row.id, row.content.unwrap_or_default());
        }
    }

    let list = rows
        .into_iter()
        .map(|row| {
            let excerpt = match row.excerpt.as_deref() {
                Some(excerpt) if !excerpt.is_empty() => excerpt.to_string(),
                _ => markdown::excerpt(
                    previews.get(&row.id).map_or("", String::as_str),
                    EXCERPT_BYTES,
                ),
            };
            let row_tags = tags.remove(&row.id).unwrap_or_default();
            PostSummaryDto::assemble(row, state.tz, excerpt, row_tags)
        })
        .collect();

    Ok(Json(PublicPostSummaryListResponse { list, page, total }))
}

/// `GET /api/posts/dates` — the publication days the calendar renders.
///
/// Bare array like `/api/categories`; `count` is available for future heatmaps
/// but the frontend currently only reads `date`. The aggregate changes only
/// when a post is published, so it is safe to cache for a minute.
pub async fn get_post_dates(
    State(state): State<SharedState>,
) -> Result<(HeaderMap, Json<Vec<PostDateCountDto>>), AppError> {
    let rows = posts::list_dates(&state.db).await.map_err(internal)?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );

    Ok((
        headers,
        Json(rows.into_iter().map(PostDateCountDto::from_row).collect()),
    ))
}

#[derive(Debug, Default, Deserialize)]
pub struct PostDetailQuery {
    edit: Option<String>,
}

/// `GET /api/posts/:slug`
pub async fn get_post(
    State(state): State<SharedState>,
    Path(post_slug): Path<String>,
    Query(query): Query<PostDetailQuery>,
) -> Result<Json<PostDto>, AppError> {
    let Some(mut row) = posts::find_by_slug(&state.db, &post_slug)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Post not found"));
    };

    // Q2b: a published post flagged is_deleted must not leak.
    if row.is_deleted.unwrap_or(false) {
        return Err(AppError::NotFound("Post not found"));
    }
    // Q3: the public route has no auth middleware in Go, so only published
    // posts are visible; `edit=true` does not unlock drafts.
    if row.status.as_deref() != Some(POST_STATUS_PUBLISHED) {
        return Err(AppError::NotFound("Post not found"));
    }

    if query.edit.as_deref() != Some("true") {
        posts::increment_view_count(&state.db, row.id)
            .await
            .map_err(internal)?;
        // Go returns the value it just wrote (`post.ViewCount + 1`) without
        // re-reading the row.
        row.view_count = Some(row.view_count.unwrap_or(0) + 1);
    }

    let relations = load_relations(&state, std::slice::from_ref(&row), true)
        .await
        .map_err(internal)?;
    Ok(Json(assemble_post(&state, row, &relations)))
}

/// `GET /api/admin/posts`
pub async fn get_admin_posts(
    State(state): State<SharedState>,
    Query(query): Query<PostListQuery>,
) -> Result<Json<AdminPostListResponse>, AppError> {
    let page = query.page.as_deref().map(parse_gin_int).unwrap_or(1);
    let page_size = query.page_size.as_deref().map(parse_gin_int).unwrap_or(20);

    let filter = PostFilter {
        published_only: false,
        hide_deleted: false,
        status: non_empty(query.status),
        category_id: non_empty(query.category_id),
        tag_id: non_empty(query.tag_id),
        keyword: non_empty(query.keyword),
        start_date: non_empty(query.start_date),
        end_date: non_empty(query.end_date),
    };

    let (limit, offset) = pagination(page, page_size);
    // Independent statements: run them together to save a database round trip.
    let (total, rows) = tokio::try_join!(
        posts::count(&state.db, &filter),
        posts::list(&state.db, &filter, limit, offset),
    )
    .map_err(internal)?;

    // Go's admin list preloads Category and Tags only; Author stays a
    // zero-value object (Q14).
    let relations = load_relations(&state, &rows, false)
        .await
        .map_err(internal)?;
    let list = rows
        .into_iter()
        .map(|row| assemble_post(&state, row, &relations))
        .collect();

    Ok(Json(AdminPostListResponse {
        list,
        page,
        page_size,
        total,
    }))
}

/// `GET /api/admin/posts/summary` — the admin post table.
///
/// Additive endpoint mirroring `GET /api/posts/summary` on the public side:
/// same auth, filters and pagination as `GET /api/admin/posts`, but it selects
/// only the columns the table renders, skips every association preload, and
/// runs the count and page queries concurrently. The article body
/// (`content`/`html_content`) never leaves MySQL, which is what made the full
/// admin list heavy.
pub async fn get_admin_posts_summary(
    State(state): State<SharedState>,
    Query(query): Query<PostListQuery>,
) -> Result<Json<AdminPostSummaryListResponse>, AppError> {
    let page = query.page.as_deref().map(parse_gin_int).unwrap_or(1);
    let page_size = query.page_size.as_deref().map(parse_gin_int).unwrap_or(20);

    let filter = PostFilter {
        published_only: false,
        hide_deleted: false,
        status: non_empty(query.status),
        category_id: non_empty(query.category_id),
        tag_id: non_empty(query.tag_id),
        keyword: non_empty(query.keyword),
        start_date: non_empty(query.start_date),
        end_date: non_empty(query.end_date),
    };

    let (limit, offset) = pagination(page, page_size);
    let (total, rows) = tokio::try_join!(
        posts::count(&state.db, &filter),
        posts::list_admin_summary(&state.db, &filter, limit, offset),
    )
    .map_err(internal)?;

    Ok(Json(AdminPostSummaryListResponse {
        list: rows
            .into_iter()
            .map(|row| AdminPostSummaryDto::from_row(row, state.tz))
            .collect(),
        page,
        page_size,
        total,
    }))
}

/// `GET /api/admin/posts/_id/:id`
pub async fn get_post_by_id(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<PostDto>, AppError> {
    let id = parse_gin_int(&raw_id);
    let row = if id > 0 {
        posts::find_by_id(&state.db, id as u64)
            .await
            .map_err(internal)?
    } else {
        None
    };
    let Some(row) = row else {
        return Err(AppError::NotFound("Post not found"));
    };

    let relations = load_relations(&state, std::slice::from_ref(&row), true)
        .await
        .map_err(internal)?;
    Ok(Json(assemble_post(&state, row, &relations)))
}

/// `GET /api/admin/posts/slug/:slug`
pub async fn get_admin_post_by_slug(
    State(state): State<SharedState>,
    Path(post_slug): Path<String>,
) -> Result<Json<PostDto>, AppError> {
    let Some(row) = posts::find_by_slug(&state.db, &post_slug)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Post not found"));
    };

    let relations = load_relations(&state, std::slice::from_ref(&row), true)
        .await
        .map_err(internal)?;
    Ok(Json(assemble_post(&state, row, &relations)))
}

#[derive(Debug, Default, Deserialize)]
struct CreatePostRequest {
    #[serde(default)]
    title: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    excerpt: String,
    #[serde(default)]
    cover_image: String,
    #[serde(default)]
    category_id: Option<u64>,
    #[serde(default)]
    tag_ids: Option<Vec<u64>>,
    #[serde(default)]
    status: String,
}

/// `POST /api/admin/posts`
pub async fn create_post(
    State(state): State<SharedState>,
    current: CurrentUser,
    body: Result<Bytes, BytesRejection>,
) -> Result<(axum::http::StatusCode, Json<PostDto>), AppError> {
    // `binding:"required"` on title and content.
    let request: CreatePostRequest = parse_body(&body)?;
    if request.title.is_empty() || request.content.is_empty() {
        return Err(AppError::BadRequest("Invalid request"));
    }
    // Go's `GetCurrentUser` can still return nil after AdminMiddleware passed
    // (e.g. the user row was soft-deleted); CreatePost then returns 401.
    let Some(author) = resolve_current_user(&state, &current).await else {
        return Err(AppError::Unauthorized("Unauthorized"));
    };

    let post_slug = if request.slug.is_empty() {
        slug::generate_slug(&request.title)
    } else {
        request.slug
    };

    if posts::slug_exists(&state.db, &post_slug)
        .await
        .map_err(internal)?
    {
        return Err(AppError::BadRequest("Slug already exists"));
    }

    let html_content = markdown::render(&request.content);
    // GORM's `default:draft` applies when the field is the zero value.
    let status = if request.status.is_empty() {
        POST_STATUS_DRAFT
    } else {
        request.status.as_str()
    };

    let id = posts::create(
        &state.db,
        &request.title,
        &post_slug,
        &request.content,
        &html_content,
        &request.excerpt,
        &request.cover_image,
        status,
        request.category_id,
        author.id,
    )
    .await
    .map_err(|err| AppError::internal_msg("Failed to create post", err))?;

    let tags_dto = if let Some(tag_ids) = request.tag_ids.as_deref() {
        if !tag_ids.is_empty() {
            let loaded = posts::tags_by_ids(&state.db, tag_ids)
                .await
                .map_err(internal)?;
            let ids: Vec<u64> = loaded.iter().map(|tag| tag.id).collect();
            posts::attach_tags(&state.db, id, &ids)
                .await
                .map_err(internal)?;
            Some(
                loaded
                    .into_iter()
                    .map(|tag| TagDto::from_row(tag, state.tz))
                    .collect(),
            )
        } else {
            None
        }
    } else {
        None
    };

    let row = posts::find_by_id(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::NotFound("Post not found"))?;
    sync_search_index(&state, None, Some(&row)).await;
    // Go returns the in-memory struct: Category/Author are zero values and
    // Tags reflect the association that was just saved.
    let dto = PostDto::assemble(
        row,
        state.tz,
        CategoryDto::zero(),
        tags_dto,
        UserDto::zero(),
    );
    Ok((axum::http::StatusCode::CREATED, Json(dto)))
}

#[derive(Debug, Default, Deserialize)]
struct UpdatePostRequest {
    #[serde(default)]
    title: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    excerpt: String,
    #[serde(default)]
    cover_image: String,
    #[serde(default)]
    category_id: Option<u64>,
    #[serde(default)]
    tag_ids: Option<Vec<u64>>,
    #[serde(default)]
    status: String,
}

/// `PUT /api/admin/posts/_id/:id`
pub async fn update_post(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<PostDto>, AppError> {
    let request: UpdatePostRequest = parse_body(&body)?;
    let id = parse_gin_int(&raw_id);
    if id <= 0 {
        return Err(AppError::NotFound("Post not found"));
    }

    let Some(mut row) = posts::find_by_id(&state.db, id as u64)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Post not found"));
    };
    // Q1: the index is keyed by slug; remember the pre-update value before
    // the request can rename the post.
    let old_slug = row.slug.clone();

    // Empty strings mean "leave unchanged" in the Go handler.
    if !request.title.is_empty() {
        row.title = request.title;
    }
    if !request.slug.is_empty() {
        row.slug = request.slug;
    }
    if !request.content.is_empty() {
        row.html_content = Some(markdown::render(&request.content));
        row.content = Some(request.content);
    }
    if !request.excerpt.is_empty() {
        row.excerpt = Some(request.excerpt);
    }
    if !request.cover_image.is_empty() {
        row.cover_image = Some(request.cover_image);
    }
    if request.category_id.is_some() {
        row.category_id = request.category_id;
    }
    if !request.status.is_empty() {
        row.status = Some(request.status);
    }

    let update_result = posts::update(
        &state.db,
        row.id,
        &row.title,
        &row.slug,
        row.content.as_deref().unwrap_or_default(),
        row.html_content.as_deref().unwrap_or_default(),
        row.excerpt.as_deref().unwrap_or_default(),
        row.cover_image.as_deref().unwrap_or_default(),
        row.status.as_deref().unwrap_or_default(),
        row.category_id,
    )
    .await;
    if let Err(err) = update_result {
        // Go discards the Save error and still returns 200.
        tracing::error!(error = ?err, post_id = row.id, "Failed to update post");
    }

    // `Association("Tags").Replace`; an empty list loads every live tag
    // because GORM's `Find(&tags, []uint{})` has no primary-key condition.
    let tags_dto = if let Some(tag_ids) = request.tag_ids {
        let loaded = if tag_ids.is_empty() {
            posts::all_tags(&state.db).await
        } else {
            posts::tags_by_ids(&state.db, &tag_ids).await
        }
        .map_err(internal)?;

        let ids: Vec<u64> = loaded.iter().map(|tag| tag.id).collect();
        if let Err(err) = posts::replace_tags(&state.db, row.id, &ids).await {
            tracing::error!(error = ?err, post_id = row.id, "Failed to replace post tags");
        }

        Some(
            loaded
                .into_iter()
                .map(|tag| TagDto::from_row(tag, state.tz))
                .collect(),
        )
    } else {
        None
    };

    sync_search_index(&state, Some(&old_slug), Some(&row)).await;

    // Go returns the in-memory post (Category/Author zero, stale timestamps).
    Ok(Json(PostDto::assemble(
        row,
        state.tz,
        CategoryDto::zero(),
        tags_dto,
        UserDto::zero(),
    )))
}

/// `DELETE /api/admin/posts/_id/:id`
pub async fn delete_post(
    State(state): State<SharedState>,
    current: CurrentUser,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    let id = parse_gin_int(&raw_id);
    let row = if id > 0 {
        posts::find_by_id(&state.db, id as u64)
            .await
            .map_err(internal)?
    } else {
        None
    };
    let Some(row) = row else {
        return Err(AppError::NotFound("Post not found"));
    };
    ensure_admin(&state, &current).await?;

    if row.status.as_deref() == Some(POST_STATUS_HIDDEN) {
        if let Err(err) = posts::hard_delete(&state.db, row.id).await {
            // GORM's Unscoped().Delete error is ignored (Q17).
            tracing::error!(error = ?err, post_id = row.id, "Failed to hard delete post");
        }
        remove_from_search(&state, &row.slug).await;
        return Ok(Json(MessageResponse {
            message: "Post permanently deleted",
        }));
    }

    let new_slug = slug::random_slug();
    if let Err(err) = posts::soft_delete(&state.db, row.id, &new_slug).await {
        tracing::error!(error = ?err, post_id = row.id, "Failed to delete post");
    }
    // Q1: remove the *old* slug; the DB row now has a random slug.
    remove_from_search(&state, &row.slug).await;
    Ok(Json(MessageResponse {
        message: "Post deleted",
    }))
}

/// `POST /api/admin/posts/_id/:id/restore`
pub async fn restore_post(
    State(state): State<SharedState>,
    current: CurrentUser,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    let id = parse_gin_int(&raw_id);
    let row = if id > 0 {
        posts::find_by_id_unscoped(&state.db, id as u64)
            .await
            .map_err(internal)?
    } else {
        None
    };
    let Some(row) = row else {
        return Err(AppError::NotFound("Post not found"));
    };
    ensure_admin(&state, &current).await?;

    if let Err(err) = posts::restore(&state.db, row.id).await {
        tracing::error!(error = ?err, post_id = row.id, "Failed to restore post");
    }
    // Q2: restore sets status=draft, so a previously indexed slug must be
    // removed (a no-op when the post was soft-deleted).
    remove_from_search(&state, &row.slug).await;
    Ok(Json(MessageResponse {
        message: "Post restored",
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct Relations {
    categories: HashMap<u64, CategoryRow>,
    authors: HashMap<u64, UserRow>,
    tags: HashMap<u64, Vec<PostTagRow>>,
}

/// Batch equivalent of `Preload("Category").Preload("Tags").Preload("Author")`.
async fn load_relations(
    state: &SharedState,
    rows: &[PostRow],
    include_author: bool,
) -> Result<Relations, sqlx::Error> {
    let category_ids: Vec<u64> = rows
        .iter()
        .filter_map(|row| row.category_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let author_ids: Vec<u64> = if include_author {
        rows.iter()
            .filter_map(|row| row.author_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    let post_ids: Vec<u64> = rows.iter().map(|row| row.id).collect();

    let categories = posts::categories_for_posts(&state.db, &category_ids).await?;
    let authors = posts::authors_for_posts(&state.db, &author_ids).await?;
    let tag_rows = posts::tags_for_posts(&state.db, &post_ids).await?;

    let mut tags: HashMap<u64, Vec<PostTagRow>> = HashMap::new();
    for tag in tag_rows {
        tags.entry(tag.post_id).or_default().push(tag);
    }

    Ok(Relations {
        categories: categories.into_iter().map(|row| (row.id, row)).collect(),
        authors: authors.into_iter().map(|row| (row.id, row)).collect(),
        tags,
    })
}

fn assemble_post(state: &SharedState, row: PostRow, relations: &Relations) -> PostDto {
    let category = row
        .category_id
        .and_then(|id| relations.categories.get(&id))
        .map(|category| CategoryDto::from_row(category.clone(), state.tz))
        .unwrap_or_else(CategoryDto::zero);

    let author = row
        .author_id
        .and_then(|id| relations.authors.get(&id))
        .map(|author| UserDto::from_row(author.clone(), state.tz))
        .unwrap_or_else(UserDto::zero);

    let tags = Some(
        relations
            .tags
            .get(&row.id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|tag| TagDto::from_post_tag_row(tag, state.tz))
            .collect(),
    );

    PostDto::assemble(row, state.tz, category, tags, author)
}

/// GORM pagination: a negative `page_size` cancels LIMIT, a negative offset
/// is ignored, and `LIMIT 0` still returns an empty list.
fn pagination(page: i64, page_size: i64) -> (i64, i64) {
    let offset = (page - 1).wrapping_mul(page_size);
    let limit = if page_size < 0 { i64::MAX } else { page_size };
    (limit, offset.max(0))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn parse_body<T: serde::de::DeserializeOwned>(
    body: &Result<Bytes, BytesRejection>,
) -> Result<T, AppError> {
    let bytes = body
        .as_ref()
        .map_err(|_| AppError::BadRequest("Invalid request"))?;
    serde_json::from_slice(bytes).map_err(|_| AppError::BadRequest("Invalid request"))
}

fn internal(err: sqlx::Error) -> AppError {
    AppError::internal_msg("Internal server error", err)
}

/// `DeletePost`/`RestorePost` re-check the **stored** role via `GetCurrentUser`
/// even though AdminMiddleware already checked the claims.
async fn ensure_admin(state: &SharedState, current: &CurrentUser) -> Result<(), AppError> {
    match resolve_current_user(state, current).await {
        Some(user) if user.role == ROLE_ADMIN => Ok(()),
        _ => Err(AppError::Forbidden("Permission denied")),
    }
}

/// Mirror a committed write into the in-memory index. Failures are logged and
/// never turn a successful HTTP response into a 500, matching Go.
async fn sync_search_index(state: &SharedState, old_slug: Option<&str>, row: Option<&PostRow>) {
    let result = match (old_slug, row) {
        (Some(old), Some(row)) if IndexedPost::is_searchable(row) => {
            search::replace_post(
                &state.search,
                old.to_string(),
                Some(IndexedPost::from_row(row)),
            )
            .await
        }
        (Some(old), _) => search::replace_post(&state.search, old.to_string(), None).await,
        (None, Some(row)) if IndexedPost::is_searchable(row) => {
            search::index_post(&state.search, IndexedPost::from_row(row)).await
        }
        _ => Ok(()),
    };

    if let Err(err) = result {
        tracing::error!(error = ?err, post_id = row.map(|row| row.id), "Failed to update search index");
    }
}

async fn remove_from_search(state: &SharedState, post_slug: &str) {
    if let Err(err) = search::delete_slug(&state.search, post_slug.to_string()).await {
        tracing::error!(error = ?err, slug = post_slug, "Failed to remove post from search index");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_matches_gorm() {
        assert_eq!(pagination(1, 10), (10, 0));
        assert_eq!(pagination(2, 10), (10, 10));
        assert_eq!(pagination(0, 0), (0, 0));
        assert_eq!(pagination(0, 20), (20, 0), "negative offset is ignored");
        assert_eq!(pagination(-1, 10), (10, 0));
        assert_eq!(
            pagination(1, -1),
            (i64::MAX, 0),
            "negative limit is cancelled"
        );
    }
}
