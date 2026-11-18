//! All SQL for `post_relations` (see migrations/0003_post_relations.sql).
//!
//! One row per relation; a `two_way` row is stored once and surfaces from
//! both sides. The public queries scope the joined post to the published
//! surface (`deleted_at IS NULL`, `is_deleted = 0`, `status = 'published'`),
//! so a relation to a draft stays invisible instead of leaking (Q2b/Q3).

use sqlx::{MySql, MySqlPool, QueryBuilder};

use crate::models::entities::{RelatedSummaryRow, RelationAdminRow, RelationRow};

/// Outgoing branch of the public related list, up to the post-id bind point:
/// relations whose `post_id` is the current post (one_way + two_way).
const RELATED_OUTGOING: &str = "\
SELECT rel.id AS rel_id, p.id, p.created_at, p.title, p.slug, p.excerpt, p.cover_image, \
p.view_count, p.category_id, c.name AS category_name \
FROM post_relations rel \
JOIN posts p ON p.id = rel.related_post_id \
LEFT JOIN categories c ON c.id = p.category_id AND c.deleted_at IS NULL \
WHERE rel.post_id = ";

/// Tail of the outgoing branch after the bind.
const RELATED_OUTGOING_TAIL: &str = " \
AND rel.relation_type IN ('one_way', 'two_way') \
AND p.deleted_at IS NULL AND p.is_deleted = 0 AND p.status = 'published' \
UNION ALL ";

/// Incoming branch, up to the post-id bind point: the reverse side of two_way
/// rows. The pair-uniqueness enforced on writes means the two branches can
/// never produce the same related post twice.
const RELATED_INCOMING: &str = "\
SELECT rel.id AS rel_id, p.id, p.created_at, p.title, p.slug, p.excerpt, p.cover_image, \
p.view_count, p.category_id, c.name AS category_name \
FROM post_relations rel \
JOIN posts p ON p.id = rel.post_id \
LEFT JOIN categories c ON c.id = p.category_id AND c.deleted_at IS NULL \
WHERE rel.related_post_id = ";

/// Tail of the incoming branch after the bind.
const RELATED_INCOMING_TAIL: &str = " \
AND rel.relation_type = 'two_way' \
AND p.deleted_at IS NULL AND p.is_deleted = 0 AND p.status = 'published'";

/// Pushes both directions of the public related list for one post.
fn push_related_union(builder: &mut QueryBuilder<MySql>, post_id: u64) {
    builder.push(RELATED_OUTGOING);
    builder.push_bind(post_id);
    builder.push(RELATED_OUTGOING_TAIL);
    builder.push(RELATED_INCOMING);
    builder.push_bind(post_id);
    builder.push(RELATED_INCOMING_TAIL);
}

/// `GET /api/posts/{slug}/related`: related cards for one post, both
/// directions, ordered by relation creation order.
pub async fn list_related(
    pool: &MySqlPool,
    post_id: u64,
    limit: i64,
    offset: i64,
) -> Result<Vec<RelatedSummaryRow>, sqlx::Error> {
    let mut builder = QueryBuilder::<MySql>::new("");
    push_related_union(&mut builder, post_id);
    builder.push(" ORDER BY rel_id ASC LIMIT ");
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);
    builder
        .build_query_as::<RelatedSummaryRow>()
        .fetch_all(pool)
        .await
}

/// Count for [`list_related`]: the same union without ordering/paging.
pub async fn count_related(pool: &MySqlPool, post_id: u64) -> Result<i64, sqlx::Error> {
    let mut builder = QueryBuilder::<MySql>::new("SELECT COUNT(*) FROM (");
    push_related_union(&mut builder, post_id);
    builder.push(") AS related");
    builder.build_query_scalar::<i64>().fetch_one(pool).await
}

/// The admin table select. LEFT JOINs keep relations to hard-deleted posts
/// visible (with empty titles) so they can still be cleaned up.
const ADMIN_SELECT: &str = "\
SELECT rel.id, rel.created_at, rel.updated_at, rel.post_id, rel.related_post_id, \
rel.relation_type, pa.title AS post_title, pa.slug AS post_slug, \
pb.title AS related_post_title, pb.slug AS related_post_slug \
FROM post_relations rel \
LEFT JOIN posts pa ON pa.id = rel.post_id \
LEFT JOIN posts pb ON pb.id = rel.related_post_id";

/// `GET /api/admin/post-relations`: all relations, optionally scoped to one
/// post, ordered by creation order.
pub async fn list(
    pool: &MySqlPool,
    post_id: Option<u64>,
    limit: i64,
    offset: i64,
) -> Result<Vec<RelationAdminRow>, sqlx::Error> {
    let mut builder = QueryBuilder::<MySql>::new(ADMIN_SELECT);
    if let Some(post_id) = post_id {
        builder.push(" WHERE rel.post_id = ");
        builder.push_bind(post_id);
    }
    builder.push(" ORDER BY rel.id ASC LIMIT ");
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);
    builder
        .build_query_as::<RelationAdminRow>()
        .fetch_all(pool)
        .await
}

/// Count for [`list`].
pub async fn count(pool: &MySqlPool, post_id: Option<u64>) -> Result<i64, sqlx::Error> {
    let mut builder = QueryBuilder::<MySql>::new("SELECT COUNT(*) FROM (");
    builder.push(ADMIN_SELECT);
    if let Some(post_id) = post_id {
        builder.push(" WHERE rel.post_id = ");
        builder.push_bind(post_id);
    }
    builder.push(") AS relations");
    builder.build_query_scalar::<i64>().fetch_one(pool).await
}

pub async fn find_by_id(pool: &MySqlPool, id: u64) -> Result<Option<RelationRow>, sqlx::Error> {
    sqlx::query_as::<_, RelationRow>(
        "SELECT id, created_at, updated_at, post_id, related_post_id, relation_type \
         FROM post_relations WHERE id = ? LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// The admin-table row for one relation (used to assemble create/update
/// responses, mirroring the list's join).
pub async fn find_admin_row_by_id(
    pool: &MySqlPool,
    id: u64,
) -> Result<Option<RelationAdminRow>, sqlx::Error> {
    let mut builder = QueryBuilder::<MySql>::new(ADMIN_SELECT);
    builder.push(" WHERE rel.id = ");
    builder.push_bind(id);
    builder.push(" LIMIT 1");
    builder
        .build_query_as::<RelationAdminRow>()
        .fetch_optional(pool)
        .await
}

/// Any row between the two posts, in either direction. One unordered pair
/// holds at most one relation: `A -> B one_way` plus `B -> A one_way` is
/// rejected in favor of a single `two_way` row.
pub async fn pair_exists(
    pool: &MySqlPool,
    post_id: u64,
    related_post_id: u64,
) -> Result<bool, sqlx::Error> {
    let found: Option<u64> = sqlx::query_scalar(
        "SELECT id FROM post_relations \
         WHERE (post_id = ? AND related_post_id = ?) \
            OR (post_id = ? AND related_post_id = ?) LIMIT 1",
    )
    .bind(post_id)
    .bind(related_post_id)
    .bind(related_post_id)
    .bind(post_id)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

pub async fn create(
    pool: &MySqlPool,
    post_id: u64,
    related_post_id: u64,
    relation_type: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO post_relations \
         (created_at, updated_at, post_id, related_post_id, relation_type) \
         VALUES (NOW(3), NOW(3), ?, ?, ?)",
    )
    .bind(post_id)
    .bind(related_post_id)
    .bind(relation_type)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id())
}

pub async fn update_relation_type(
    pool: &MySqlPool,
    id: u64,
    relation_type: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE post_relations SET relation_type = ?, updated_at = NOW(3) WHERE id = ?")
        .bind(relation_type)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Relations are deleted physically: they carry no content of their own.
pub async fn delete(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM post_relations WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
