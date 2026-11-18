-- Widen `html_content` from TEXT (65,535 bytes) to MEDIUMTEXT (16 MiB).
--
-- Long articles overflowed TEXT: with strict mode MySQL rejects the
-- INSERT/UPDATE with error 1406 ("Data too long for column 'html_content'"),
-- which surfaced as a 500 on post/page create or update. The Markdown source
-- (`content`) is always smaller than its rendered HTML, so it keeps its TEXT
-- type and the API's existing 2 MiB body limit still applies upstream.
--
-- Both public content tables are widened in the same migration. Changing a
-- TEXT-family column type requires a table rebuild (ALGORITHM=COPY), which is
-- why the two ALTERs are simple and unconditional; sqlx records this file in
-- `_sqlx_migrations`, so it runs exactly once per database.

ALTER TABLE posts MODIFY COLUMN html_content MEDIUMTEXT;

ALTER TABLE pages MODIFY COLUMN html_content MEDIUMTEXT;
