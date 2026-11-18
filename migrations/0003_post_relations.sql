-- Post relations: manually curated "related posts" links between articles
-- (see `handlers/post_relations.rs`).
--
-- One row per relation. `relation_type` is `one_way` (only `post_id`'s page
-- lists `related_post_id`) or `two_way` (both pages list each other; the row
-- is shared, no reverse duplicate is stored). Display order follows `id`
-- (creation order). No deleted_at: relations are deleted physically.

CREATE TABLE IF NOT EXISTS post_relations (
    id              BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    created_at      DATETIME(3) DEFAULT NULL,
    updated_at      DATETIME(3) DEFAULT NULL,
    post_id         BIGINT UNSIGNED NOT NULL,
    related_post_id BIGINT UNSIGNED NOT NULL,
    relation_type   VARCHAR(20) NOT NULL DEFAULT 'one_way',
    PRIMARY KEY (id),
    UNIQUE INDEX idx_post_relations_pair (post_id, related_post_id),
    INDEX idx_post_relations_related (related_post_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;
