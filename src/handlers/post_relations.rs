//! Post relation endpoints: the public "related posts" list and the admin
//! CRUD for curated relations (`post_relations`, see migration 0003).
//!
//! A relation is `one_way` (only `post_id`'s page lists `related_post_id`) or
//! `two_way` (both pages list each other; the row is shared, so one unordered
//! pair of posts holds at most one relation). Additive surface — no Go
//! baseline contract to preserve.

use std::collections::HashMap;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{
    MessageResponse, POST_STATUS_PUBLISHED, PostRelationDto, PostRelationListResponse,
    PostSummaryDto, PostSummaryTagDto, RelatedPostListResponse, RelatedSummaryRow,
};
use crate::repo::{post_relations, posts};
use crate::services::markdown;
use crate::services::params::parse_gin_int;
use crate::state::SharedState;

const EXCERPT_BYTES: usize = 200;

pub const RELATION_ONE_WAY: &str = "one_way";
pub const RELATION_TWO_WAY: &str = "two_way";

#[derive(Debug, Default, Deserialize)]
pub struct RelatedListQuery {
    page: Option<String>,
    page_size: Option<String>,
}

/// `GET /api/posts/{slug}/related`
///
/// The curated related-posts list for one published post. `page_size`
/// defaults to 5 (the detail-page widget); the "view all" page passes a
/// larger value. Visibility mirrors `get_post` (Q2b/Q3): both the requested
/// post and every related post must be published and not deleted, so drafts
/// never leak through relations.
pub async fn get_related_posts(
    State(state): State<SharedState>,
    Path(post_slug): Path<String>,
    Query(query): Query<RelatedListQuery>,
) -> Result<Json<RelatedPostListResponse>, AppError> {
    let Some(row) = posts::find_by_slug(&state.db, &post_slug)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Post not found"));
    };
    if row.is_deleted.unwrap_or(false) {
        return Err(AppError::NotFound("Post not found"));
    }
    if row.status.as_deref() != Some(POST_STATUS_PUBLISHED) {
        return Err(AppError::NotFound("Post not found"));
    }

    let page = query.page.as_deref().map(parse_gin_int).unwrap_or(1);
    let page_size = query.page_size.as_deref().map(parse_gin_int).unwrap_or(5);
    let (limit, offset) = pagination(page, page_size);

    let (total, rows) = tokio::try_join!(
        post_relations::count_related(&state.db, row.id),
        post_relations::list_related(&state.db, row.id, limit, offset),
    )
    .map_err(internal)?;

    let list = assemble_summaries(&state, rows).await?;
    Ok(Json(RelatedPostListResponse { list, page, total }))
}

/// Summary-card assembly shared with `get_posts_summary`: batch-load tags and
/// derive an excerpt from the body when the stored one is empty.
async fn assemble_summaries(
    state: &SharedState,
    rows: Vec<RelatedSummaryRow>,
) -> Result<Vec<PostSummaryDto>, AppError> {
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
            PostSummaryDto::assemble(row.into(), state.tz, excerpt, row_tags)
        })
        .collect();

    Ok(list)
}

#[derive(Debug, Default, Deserialize)]
pub struct RelationAdminQuery {
    page: Option<String>,
    page_size: Option<String>,
    post_id: Option<String>,
}

/// `GET /api/admin/post-relations`
pub async fn get_admin_post_relations(
    State(state): State<SharedState>,
    Query(query): Query<RelationAdminQuery>,
) -> Result<Json<PostRelationListResponse>, AppError> {
    let page = query.page.as_deref().map(parse_gin_int).unwrap_or(1);
    let page_size = query.page_size.as_deref().map(parse_gin_int).unwrap_or(20);
    let post_id = query
        .post_id
        .as_deref()
        .map(parse_gin_int)
        .filter(|id| *id > 0)
        .map(|id| id as u64);

    let (limit, offset) = pagination(page, page_size);
    let (total, rows) = tokio::try_join!(
        post_relations::count(&state.db, post_id),
        post_relations::list(&state.db, post_id, limit, offset),
    )
    .map_err(internal)?;

    let list = rows
        .into_iter()
        .map(|row| PostRelationDto::from_row(row, state.tz))
        .collect();
    Ok(Json(PostRelationListResponse {
        list,
        page,
        page_size,
        total,
    }))
}

#[derive(Debug, Default, Deserialize)]
struct CreatePostRelationRequest {
    #[serde(default)]
    post_id: u64,
    #[serde(default)]
    related_post_id: u64,
    #[serde(default)]
    relation_type: String,
}

/// `POST /api/admin/post-relations`
///
/// Both posts must exist (any status, so relations can be curated before a
/// draft goes live); duplicates are rejected in both directions. The reverse
/// direction of a two_way row stays unique via the same check plus the
/// `(post_id, related_post_id)` unique index.
pub async fn create_post_relation(
    State(state): State<SharedState>,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<PostRelationDto>), AppError> {
    let request: CreatePostRelationRequest = parse_body(&body)?;
    if request.post_id == 0 || request.related_post_id == 0 {
        return Err(AppError::BadRequest("Invalid request"));
    }
    if request.post_id == request.related_post_id {
        return Err(AppError::BadRequest("Invalid request"));
    }
    let relation_type = if request.relation_type.is_empty() {
        RELATION_ONE_WAY
    } else {
        validate_relation_type(&request.relation_type)?
    };

    for post_id in [request.post_id, request.related_post_id] {
        if posts::find_by_id(&state.db, post_id)
            .await
            .map_err(internal)?
            .is_none()
        {
            return Err(AppError::NotFound("Post not found"));
        }
    }
    if post_relations::pair_exists(&state.db, request.post_id, request.related_post_id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::Conflict("Relation already exists"));
    }

    let id = post_relations::create(
        &state.db,
        request.post_id,
        request.related_post_id,
        relation_type,
    )
    .await
    .map_err(|err| {
        // Two concurrent creates can pass the pair check; the unique index
        // turns the race into a conflict instead of a leaked 1062 error.
        if is_unique_violation(&err) {
            AppError::Conflict("Relation already exists")
        } else {
            AppError::internal_msg("Failed to create relation", err)
        }
    })?;

    let row = post_relations::find_admin_row_by_id(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::NotFound("Relation not found"))?;

    Ok((
        StatusCode::CREATED,
        Json(PostRelationDto::from_row(row, state.tz)),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct UpdatePostRelationRequest {
    #[serde(default)]
    relation_type: String,
}

/// `PUT /api/admin/post-relations/_id/{id}` — flip one_way/two_way.
///
/// No reverse-side check is needed: one unordered pair holds at most one
/// relation, so switching to two_way can never collide.
pub async fn update_post_relation(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<PostRelationDto>, AppError> {
    let id = parse_gin_int(&raw_id);
    if id <= 0 {
        return Err(AppError::NotFound("Relation not found"));
    }

    let request: UpdatePostRelationRequest = parse_body(&body)?;
    let relation_type = validate_relation_type(&request.relation_type)?;

    let Some(row) = post_relations::find_by_id(&state.db, id as u64)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Relation not found"));
    };

    post_relations::update_relation_type(&state.db, row.id, relation_type)
        .await
        .map_err(|err| AppError::internal_msg("Failed to update relation", err))?;

    let row = post_relations::find_admin_row_by_id(&state.db, row.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::NotFound("Relation not found"))?;

    Ok(Json(PostRelationDto::from_row(row, state.tz)))
}

/// `DELETE /api/admin/post-relations/_id/{id}` — physical delete.
pub async fn delete_post_relation(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    let id = parse_gin_int(&raw_id);
    if id > 0 {
        post_relations::delete(&state.db, id as u64)
            .await
            .map_err(|err| AppError::internal_msg("Failed to delete relation", err))?;
    }
    Ok(Json(MessageResponse {
        message: "Relation deleted",
    }))
}

fn validate_relation_type(raw: &str) -> Result<&'static str, AppError> {
    match raw {
        RELATION_ONE_WAY => Ok(RELATION_ONE_WAY),
        RELATION_TWO_WAY => Ok(RELATION_TWO_WAY),
        _ => Err(AppError::BadRequest("Invalid request")),
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .is_some_and(|db| db.is_unique_violation())
}

/// GORM pagination semantics (mirrors `handlers/posts.rs::pagination`).
fn pagination(page: i64, page_size: i64) -> (i64, i64) {
    let offset = (page - 1).wrapping_mul(page_size);
    let limit = if page_size < 0 { i64::MAX } else { page_size };
    (limit, offset.max(0))
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
