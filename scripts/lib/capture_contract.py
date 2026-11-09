#!/usr/bin/env python3
"""Capture the HTTP contract of the frozen Go baseline.

Every request/response/email/note is written as a normalized JSON fixture.
Run through scripts/capture_contract.sh (it starts MySQL/Redis/MailHog, the
Go server and the TTS mock).
"""
from __future__ import annotations

import base64
import json
import os
import pathlib
import re
import sys
import urllib.parse

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from capture_common import (  # noqa: E402
    CAPTURE_CLIENT_IP,
    Capture,
    MailHog,
    RedisClient,
    env,
    file_sha256,
    http_request,
    make_jwt,
)

# 1x1 transparent PNG.
TINY_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
)

# Fixed claim times keep captured JWTs reproducible (2025-10-09 .. 2033-05-18).
ISSUED_AT = 1_760_000_000
EXPIRES_AT = 2_000_000_000

BOUNDARY = "----smileyan-capture-boundary"

TIME_OFFSET = re.compile(r"([+-]\d{2}:\d{2}|Z)$")
TIME_RULE = "Go RFC3339Nano: trailing-zero-trimmed fraction, omitted when zero (DATETIME(3) => 0-3 digits)"


def time_offset(value: str) -> str:
    """Return only the timezone offset so the fixture is deterministic."""
    match = TIME_OFFSET.search(value)
    return match.group(1) if match else "unknown"


def q(path: str, **params) -> str:
    return f"{path}?{urllib.parse.urlencode(params)}"


def expect(result, status: int, name: str):
    actual, _, body, parsed = result
    if actual != status:
        preview = body[:500].decode("utf-8", "replace")
        raise AssertionError(f"{name}: expected HTTP {status}, got {actual}: {preview}")
    return parsed


def multipart_body(field: str, filename: str, content: bytes, content_type: str):
    body = (
        f"--{BOUNDARY}\r\n"
        f'Content-Disposition: form-data; name="{field}"; filename="{filename}"\r\n'
        f"Content-Type: {content_type}\r\n\r\n"
    ).encode() + content + f"\r\n--{BOUNDARY}--\r\n".encode()
    meta = {"field": field, "filename": filename, "bytes": len(content)}
    return body, f"multipart/form-data; boundary={BOUNDARY}", meta


def multipart_no_file(field: str):
    body = (
        f"--{BOUNDARY}\r\n"
        f'Content-Disposition: form-data; name="{field}"\r\n\r\n'
        f"x\r\n--{BOUNDARY}--\r\n"
    ).encode()
    return body, f"multipart/form-data; boundary={BOUNDARY}", {"field": field, "file": False}


def tts_last(tts_base: str, cap: Capture, name: str) -> None:
    status, _, body = http_request("GET", f"{tts_base.rstrip('/')}/__last")
    if status != 200:
        raise AssertionError(f"{name}: mock TTS /__last returned {status}")
    cap.note(name, json.loads(body))


def main() -> None:
    base_url = env("CAPTURE_BASE_URL", "http://127.0.0.1:18080")
    out_dir = pathlib.Path(env("CAPTURE_OUT_DIR", "tests/fixtures/contract"))
    mailhog_url = env("CAPTURE_MAILHOG_URL", "http://127.0.0.1:18025")
    tts_base = env("CAPTURE_TTS_MOCK_URL", "http://127.0.0.1:19099")
    jwt_secret = env("CAPTURE_JWT_SECRET", "capture-jwt-secret")
    dev_token = env("CAPTURE_DEV_TOKEN", "capture-dev-token")
    admin_email = env("CAPTURE_ADMIN_EMAIL", "admin@example.com")
    reader_email = env("CAPTURE_READER_EMAIL", "reader@example.com")

    common_claims = {"iat": ISSUED_AT, "nbf": ISSUED_AT, "exp": EXPIRES_AT}
    tokens = {
        "admin": make_jwt(
            {"user_id": 1, "email": admin_email, "role": "admin", **common_claims},
            jwt_secret,
        ),
        "reader": make_jwt(
            {"user_id": 2, "email": reader_email, "role": "reader", **common_claims},
            jwt_secret,
        ),
        "dev": make_jwt({"dev": dev_token, **common_claims}, jwt_secret),
        "admin_hs512": make_jwt(
            {"user_id": 1, "email": admin_email, "role": "admin", **common_claims},
            jwt_secret,
            alg="HS512",
        ),
    }

    mailhog = MailHog(mailhog_url)
    mailhog.clear()
    redis = RedisClient(
        env("CAPTURE_REDIS_HOST", "127.0.0.1"),
        int(env("CAPTURE_REDIS_PORT", "16379")),
        env("CAPTURE_REDIS_PASSWORD", "capturepass"),
    )
    redis.cmd("PING")

    cap = Capture(out_dir, base_url, tokens, mailhog=mailhog, redis=redis)

    schema = pathlib.Path(env("CAPTURE_SCHEMA", "tests/fixtures/db/0001_schema.sql"))
    seed = pathlib.Path(env("CAPTURE_SEED", "tests/fixtures/db/seed.sql"))
    meta = {
        "capture_client_ip": CAPTURE_CLIENT_IP,
        "backend_commit": env("CAPTURE_BACKEND_COMMIT", "unknown"),
        "backend_tag": env("CAPTURE_BACKEND_TAG", ""),
        "schema_sha256": file_sha256(schema) if schema.exists() else "unknown",
        "seed_sha256": file_sha256(seed) if seed.exists() else "unknown",
    }

    # =====================================================================
    # A. Public reads (no auth)
    # =====================================================================
    cap.call("hello", "GET", "/api/hello")

    cap.call("posts_default", "GET", "/api/posts")
    cap.call("posts_page2_size2", "GET", q("/api/posts", page=2, page_size=2))
    cap.call("posts_category", "GET", q("/api/posts", category_id=1))
    cap.call("posts_tag", "GET", q("/api/posts", tag_id=1))
    cap.call("posts_keyword", "GET", q("/api/posts", keyword="Published"))
    cap.call(
        "posts_date_range",
        "GET",
        q("/api/posts", start_date="2026-09-01", end_date="2026-09-30"),
    )
    cap.call("posts_invalid_page", "GET", q("/api/posts", page="abc", page_size="xyz"))
    cap.call("posts_page_size_zero", "GET", q("/api/posts", page=1, page_size=0))
    cap.call("posts_negative_page", "GET", q("/api/posts", page=-1, page_size=10))

    _, _, _, published_post = cap.call("post_detail_published", "GET", "/api/posts/published-post")
    cap.note(
        "time_format_sample_seeded",
        {
            "created_at": published_post["created_at"],
            "offset": time_offset(published_post["created_at"]),
            "rule": TIME_RULE,
        },
    )
    cap.call("post_detail_draft_public", "GET", "/api/posts/draft-post")
    cap.call("post_detail_draft_edit_public", "GET", q("/api/posts/draft-post", edit="true"))
    cap.call(
        "post_detail_draft_edit_admin_token",
        "GET",
        q("/api/posts/draft-post", edit="true"),
        auth="admin",
    )
    cap.call("post_detail_hidden", "GET", "/api/posts/hidden-post")
    cap.call("post_detail_is_deleted", "GET", "/api/posts/deletedpost")
    cap.call("post_detail_published_is_deleted_leak", "GET", "/api/posts/published-but-is-deleted")
    cap.call("post_detail_soft_deleted", "GET", "/api/posts/soft-deleted-at-post")
    cap.call("post_detail_missing", "GET", "/api/posts/no-such-post")

    cap.call("categories", "GET", "/api/categories")
    cap.call("tags", "GET", "/api/tags")

    cap.call("pages_default", "GET", "/api/pages")
    cap.call("pages_parse_int_quirk", "GET", q("/api/pages", page="1abc", page_size="2xyz"))
    cap.call("pages_page_size_zero", "GET", q("/api/pages", page=1, page_size=0))
    cap.call("page_detail_published", "GET", "/api/pages/about")
    cap.call("page_detail_draft_public", "GET", "/api/pages/draft-page")
    cap.call("page_detail_deleted", "GET", "/api/pages/legacy-page")
    cap.call("page_detail_missing", "GET", "/api/pages/no-such-page")

    cap.call("comments_by_post", "GET", q("/api/comments", post_id=1))
    cap.call("comments_all", "GET", "/api/comments")
    cap.call("comments_draft_post", "GET", q("/api/comments", post_id=2))
    cap.call("comments_missing_post", "GET", q("/api/comments", post_id=999))

    cap.call("search_missing_keyword", "GET", "/api/search")
    cap.call("search_empty_keyword", "GET", "/api/search?keyword=")
    cap.call("search_chinese_short", "GET", q("/api/search", keyword="文"))
    cap.call("search_english", "GET", q("/api/search", keyword="September"))
    cap.call("search_draft_leak", "GET", q("/api/search", keyword="zzqdraft"))
    cap.call("search_hidden_leak", "GET", q("/api/search", keyword="zzqhidden"))
    cap.call("search_deleted_leak", "GET", q("/api/search", keyword="zzqdeleted"))
    cap.call(
        "search_published_is_deleted_leak",
        "GET",
        q("/api/search", keyword="zzqpubdel"),
    )
    cap.call("search_soft_deleted_at", "GET", q("/api/search", keyword="zzqsoft"))
    cap.call("search_paging", "GET", q("/api/search", keyword="文章", page=2, page_size=1))
    cap.call("search_plus_operator", "GET", q("/api/search", keyword="+Go"))

    cap.call("subscribe_invalid_email", "POST", "/api/subscribe", json_body={"email": "not-an-email"})
    expect(
        cap.call("subscribe_new", "POST", "/api/subscribe", json_body={"email": "newsub@example.com"}),
        200,
        "subscribe_new",
    )
    cap.email("email_subscribe_confirmation", "newsub@example.com")
    cap.call(
        "subscribe_duplicate",
        "POST",
        "/api/subscribe",
        json_body={"email": "newsub@example.com"},
    )
    cap.call(
        "subscribe_soft_deleted_duplicate",
        "POST",
        "/api/subscribe",
        json_body={"email": "deleted-sub@example.com"},
    )
    cap.call("unsubscribe_missing_email", "GET", "/api/unsubscribe")
    cap.call(
        "unsubscribe_wrong_token",
        "GET",
        q("/api/unsubscribe", email="a@example.com", token="wrong"),
    )
    cap.call(
        "unsubscribe_ok",
        "GET",
        q(
            "/api/unsubscribe",
            email="a@example.com",
            token="0123456789abcdef0123456789abcdef",
        ),
    )
    cap.call("unsubscribe_without_token", "GET", q("/api/unsubscribe", email="b@example.com"))

    cap.call("method_not_allowed", "DELETE", "/api/hello")
    cap.call("not_found", "GET", "/api/no-such-route")
    cap.call("trailing_slash", "GET", "/api/posts/")
    cap.call(
        "cors_preflight",
        "OPTIONS",
        "/api/posts",
        extra_headers={
            "Origin": "https://example.com",
            "Access-Control-Request-Method": "GET",
            "Access-Control-Request-Headers": "authorization,content-type",
        },
    )
    cap.call("cors_simple_get", "GET", "/api/hello", extra_headers={"Origin": "https://example.com"})

    # =====================================================================
    # B. Auth / users / uploads
    # =====================================================================
    cap.call("admin_route_no_header", "GET", "/api/admin/posts")
    cap.call(
        "admin_route_bad_format",
        "GET",
        "/api/admin/posts",
        extra_headers={"Authorization": "Token abc"},
    )
    cap.call(
        "admin_route_invalid_token",
        "GET",
        "/api/admin/posts",
        extra_headers={"Authorization": "Bearer not-a-jwt"},
    )
    cap.call("admin_route_reader_token", "GET", "/api/admin/posts", auth="reader")
    cap.call("admin_route_admin_token", "GET", "/api/admin/posts", auth="admin")
    cap.call("admin_route_hs512_token", "GET", "/api/admin/posts", auth="admin_hs512")
    cap.call("user_get_admin_token", "GET", "/api/user", auth="admin")
    cap.call("user_get_dev_token", "GET", "/api/user", auth="dev")
    cap.call("user_get_no_token", "GET", "/api/user")

    cap.call("send_code_invalid_email", "POST", "/api/send-code", json_body={"email": "nope"})
    cap.call("send_code_reader", "POST", "/api/send-code", json_body={"email": reader_email})
    reader_code = MailHog.code(mailhog.wait_for(reader_email))
    cap.email("email_verification_reader", reader_email)
    cap.redis_snapshot("redis_after_send_code_reader", reader_email)

    cap.call("send_code_reader_again", "POST", "/api/send-code", json_body={"email": reader_email})
    cap.redis_snapshot("redis_after_send_code_reader_again", reader_email)

    cap.call("send_code_admin_bypass", "POST", "/api/send-code", json_body={"email": admin_email})
    cap.email("email_verification_admin", admin_email)
    # Clear the mailbox so wait_for() cannot return the first (now overwritten) code.
    mailhog.clear()
    cap.call("send_code_admin_again", "POST", "/api/send-code", json_body={"email": admin_email})
    admin_code = MailHog.code(mailhog.wait_for(admin_email))
    cap.email("email_verification_admin_again", admin_email)
    cap.redis_snapshot("redis_after_send_code_admin", admin_email)

    cap.call("login_invalid_email", "POST", "/api/login", json_body={"email": "nope", "code": "123456"})
    cap.call("login_short_code", "POST", "/api/login", json_body={"email": reader_email, "code": "123"})
    wrong_code = "000000" if reader_code != "000000" else "111111"
    cap.call(
        "login_wrong_code",
        "POST",
        "/api/login",
        json_body={"email": reader_email, "code": wrong_code},
    )
    login_reader = expect(
        cap.call(
            "login_reader",
            "POST",
            "/api/login",
            json_body={"email": reader_email, "code": reader_code},
        ),
        200,
        "login_reader",
    )
    tokens["reader_login"] = login_reader["token"]
    login_admin = expect(
        cap.call(
            "login_admin",
            "POST",
            "/api/login",
            json_body={"email": admin_email, "code": admin_code},
        ),
        200,
        "login_admin",
    )
    tokens["admin_login"] = login_admin["token"]

    short_email = "a@b.co"
    cap.call("send_code_short_email", "POST", "/api/send-code", json_body={"email": short_email})
    short_code = MailHog.code(mailhog.wait_for(short_email))
    cap.email("email_verification_short_email", short_email)
    cap.call(
        "login_short_email_panic",
        "POST",
        "/api/login",
        json_body={"email": short_email, "code": short_code},
    )

    new_email = "newuser@example.com"
    cap.call("send_code_new_user", "POST", "/api/send-code", json_body={"email": new_email})
    new_code = MailHog.code(mailhog.wait_for(new_email))
    cap.email("email_verification_new_user", new_email)
    login_new = expect(
        cap.call(
            "login_new_user",
            "POST",
            "/api/login",
            json_body={"email": new_email, "code": new_code},
        ),
        200,
        "login_new_user",
    )
    tokens["new_user"] = login_new["token"]
    cap.call("user_get_new_user", "GET", "/api/user", auth="new_user")

    cap.call("user_get_reader", "GET", "/api/user", auth="reader")
    cap.call("user_put_reader", "PUT", "/api/user", auth="reader", json_body={"nickname": "Reader Renamed"})

    body, content_type, meta_upload = multipart_no_file("other")
    cap.call(
        "upload_avatar_no_file",
        "POST",
        "/api/upload-avatar",
        auth="reader",
        raw_body=body,
        content_type=content_type,
        multipart={"field": "avatar", "file": False},
    )
    body, content_type, meta_upload = multipart_body("avatar", "capture-avatar.txt", b"not an image", "text/plain")
    cap.call(
        "upload_avatar_txt",
        "POST",
        "/api/upload-avatar",
        auth="reader",
        raw_body=body,
        content_type=content_type,
        multipart=meta_upload,
    )
    body, content_type, meta_upload = multipart_body("avatar", "capture-avatar.jpeg", TINY_PNG, "image/png")
    cap.call(
        "upload_avatar_jpeg_ext",
        "POST",
        "/api/upload-avatar",
        auth="reader",
        raw_body=body,
        content_type=content_type,
        multipart=meta_upload,
    )
    body, content_type, meta_upload = multipart_body("avatar", "capture-avatar.png", TINY_PNG, "image/png")
    upload = expect(
        cap.call(
            "upload_avatar_ok",
            "POST",
            "/api/upload-avatar",
            auth="reader",
            raw_body=body,
            content_type=content_type,
            multipart=meta_upload,
        ),
        200,
        "upload_avatar_ok",
    )
    cap.call("get_uploaded_avatar", "GET", upload["avatar"])

    cap.call(
        "create_comment_no_auth",
        "POST",
        "/api/comments",
        json_body={"post_id": 1, "content": "no auth"},
    )
    cap.call(
        "create_comment_missing_post",
        "POST",
        "/api/comments",
        auth="reader",
        json_body={"post_id": 999, "content": "missing post"},
    )
    comment = expect(
        cap.call(
            "create_comment_reader",
            "POST",
            "/api/comments",
            auth="reader",
            json_body={"post_id": 1, "content": "契约测试评论 <b>html</b>"},
        ),
        201,
        "create_comment_reader",
    )
    # Clear the mailbox so the reply notification is unambiguously the newest
    # message for admin@example.com.
    mailhog.clear()
    expect(
        cap.call(
            "create_comment_reply",
            "POST",
            "/api/comments",
            auth="reader",
            json_body={"post_id": 1, "content": "回复管理员评论", "parent_id": 2},
        ),
        201,
        "create_comment_reply",
    )
    cap.email("email_comment_reply_notification", admin_email)
    cap.note("created_comment_id", {"id": comment["id"]})

    # =====================================================================
    # C. TTS proxy
    # =====================================================================
    tts_base_body = {
        "model": "tts-capture",
        "input": "你好，契约测试",
        "voice": "alloy",
        "response_format": "mp3",
    }

    cap.call("tts_no_auth", "POST", "/v1/audio/speech", json_body=tts_base_body)
    cap.call(
        "tts_invalid_json",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        raw_body=b'{"input":',
        content_type="application/json",
    )
    cap.call(
        "tts_empty_input",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "input": "   "},
    )
    cap.call(
        "tts_input_too_long_ascii",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "input": "a" * 4097},
    )
    cap.call(
        "tts_input_cjk_byte_quirk",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "input": "中" * 1366},
    )
    cap.call(
        "tts_missing_voice",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={"model": "tts-capture", "input": "no voice"},
    )
    cap.call(
        "tts_empty_voice",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "voice": ""},
    )
    cap.call(
        "tts_bad_voice_object",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "voice": {"foo": "bar"}},
    )
    cap.call(
        "tts_bad_format",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "response_format": "ogg"},
    )
    cap.call(
        "tts_bad_speed_low",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "speed": 0.1},
    )
    cap.call(
        "tts_bad_speed_high",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "speed": 4.1},
    )

    expect(
        cap.call(
            "tts_binary",
            "POST",
            "/v1/audio/speech",
            auth="reader",
            json_body={**tts_base_body, "input": "中" * 1365},
        ),
        200,
        "tts_binary",
    )
    tts_last(tts_base, cap, "tts_upstream_binary")

    expect(
        cap.call(
            "tts_voice_object_ok",
            "POST",
            "/v1/audio/speech",
            auth="reader",
            json_body={**tts_base_body, "voice": {"id": "alloy"}},
        ),
        200,
        "tts_voice_object_ok",
    )
    tts_last(tts_base, cap, "tts_upstream_voice_object")

    expect(
        cap.call(
            "tts_model_default",
            "POST",
            "/v1/audio/speech",
            auth="reader",
            json_body={"input": "default model", "voice": "alloy"},
        ),
        200,
        "tts_model_default",
    )
    tts_last(tts_base, cap, "tts_upstream_model_default")

    expect(
        cap.call(
            "tts_instructions_speed",
            "POST",
            "/v1/audio/speech",
            auth="reader",
            json_body={**tts_base_body, "speed": 1.5, "instructions": "calm voice"},
        ),
        200,
        "tts_instructions_speed",
    )
    tts_last(tts_base, cap, "tts_upstream_instructions_speed")

    expect(
        cap.call(
            "tts_sse",
            "POST",
            "/v1/audio/speech",
            auth="reader",
            json_body={**tts_base_body, "stream_format": "sse"},
        ),
        200,
        "tts_sse",
    )
    tts_last(tts_base, cap, "tts_upstream_sse")

    cap.call(
        "tts_upstream_error_400",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "input": "TRIGGER_ERROR_400"},
    )
    cap.call(
        "tts_upstream_error_500",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "input": "TRIGGER_ERROR_500"},
    )
    cap.call(
        "tts_upstream_drop",
        "POST",
        "/v1/audio/speech",
        auth="reader",
        json_body={**tts_base_body, "input": "TRIGGER_DROP"},
    )

    # =====================================================================
    # D. Admin: reads
    # =====================================================================
    cap.call("admin_posts_default", "GET", "/api/admin/posts", auth="admin")
    cap.call("admin_posts_published", "GET", q("/api/admin/posts", status="published"), auth="admin")
    cap.call("admin_posts_hidden", "GET", q("/api/admin/posts", status="hidden"), auth="admin")
    cap.call("admin_posts_category", "GET", q("/api/admin/posts", category_id=1), auth="admin")
    cap.call("admin_posts_tag", "GET", q("/api/admin/posts", tag_id=1), auth="admin")
    cap.call("admin_posts_keyword", "GET", q("/api/admin/posts", keyword="Published"), auth="admin")
    cap.call(
        "admin_posts_invalid_page",
        "GET",
        q("/api/admin/posts", page="abc", page_size="xyz"),
        auth="admin",
    )
    cap.call("admin_post_by_id_1", "GET", "/api/admin/posts/_id/1", auth="admin")
    cap.call("admin_post_by_id_is_deleted_4", "GET", "/api/admin/posts/_id/4", auth="admin")
    cap.call("admin_post_by_id_soft_deleted_5", "GET", "/api/admin/posts/_id/5", auth="admin")
    cap.call("admin_post_by_id_missing", "GET", "/api/admin/posts/_id/9999", auth="admin")
    cap.call("admin_post_by_slug", "GET", "/api/admin/posts/slug/published-post", auth="admin")
    cap.call("admin_post_by_slug_missing", "GET", "/api/admin/posts/slug/nope", auth="admin")
    cap.call("admin_comments_default", "GET", "/api/admin/comments", auth="admin")
    cap.call("admin_comments_pending", "GET", q("/api/admin/comments", status="pending"), auth="admin")
    cap.call("admin_subscriptions", "GET", "/api/admin/subscriptions", auth="admin")
    cap.call("admin_users", "GET", "/api/admin/users", auth="admin")

    # =====================================================================
    # E. Admin: writes (ids start at 100 per seed)
    # =====================================================================
    category = expect(
        cap.call(
            "admin_create_category",
            "POST",
            "/api/admin/categories",
            auth="admin",
            json_body={"name": "契约分类", "description": "capture", "sort": 9},
        ),
        201,
        "admin_create_category",
    )
    category_id = category["id"]
    expect(
        cap.call(
            "admin_update_category",
            "PUT",
            f"/api/admin/categories/_id/{category_id}",
            auth="admin",
            json_body={"name": "契约分类更新", "description": "updated", "sort": 5},
        ),
        200,
        "admin_update_category",
    )

    tag = expect(
        cap.call(
            "admin_create_tag",
            "POST",
            "/api/admin/tags",
            auth="admin",
            json_body={"name": "契约标签"},
        ),
        201,
        "admin_create_tag",
    )
    tag_id = tag["id"]

    page = expect(
        cap.call(
            "admin_create_page_published",
            "POST",
            "/api/admin/pages",
            auth="admin",
            json_body={"title": "契约页面", "content": "# 契约页面\n", "status": "published"},
        ),
        201,
        "admin_create_page_published",
    )
    page_id = page["id"]
    draft_page = expect(
        cap.call(
            "admin_create_page_draft",
            "POST",
            "/api/admin/pages",
            auth="admin",
            json_body={"title": "契约草稿页面", "content": "# 草稿\n", "status": "draft"},
        ),
        201,
        "admin_create_page_draft",
    )
    draft_page_id = draft_page["id"]
    expect(
        cap.call(
            "admin_update_page",
            "PUT",
            f"/api/admin/pages/_id/{page_id}",
            auth="admin",
            json_body={"title": "契约页面更新", "content": "# 更新页面\n", "status": "published"},
        ),
        200,
        "admin_update_page",
    )

    post = expect(
        cap.call(
            "admin_create_post",
            "POST",
            "/api/admin/posts",
            auth="admin",
            json_body={
                "title": "契约测试文章 Contract Capture",
                "content": "# 契约测试\n\n用于采集创建/更新/删除/恢复的响应。\n",
                "excerpt": "",
                "cover_image": "",
                "category_id": 1,
                "tag_ids": [1, 2],
                "status": "published",
            },
        ),
        201,
        "admin_create_post",
    )
    post_id = post["id"]
    cap.note("created_post", {"id": post_id, "slug": post["slug"]})
    cap.note(
        "time_format_sample_created",
        {
            "offset": time_offset(post["created_at"]),
            "rule": TIME_RULE,
        },
    )

    cap.call(
        "admin_create_post_duplicate_slug",
        "POST",
        "/api/admin/posts",
        auth="admin",
        json_body={
            "title": "Duplicate",
            "slug": "published-post",
            "content": "duplicate",
            "status": "published",
        },
    )
    cap.call(
        "admin_create_post_missing_title",
        "POST",
        "/api/admin/posts",
        auth="admin",
        json_body={"content": "missing title"},
    )
    expect(
        cap.call(
            "admin_update_post",
            "PUT",
            f"/api/admin/posts/_id/{post_id}",
            auth="admin",
            json_body={
                "title": "契约测试文章 Contract Capture Updated",
                "content": "# 更新后\n\n更新内容。\n",
                "excerpt": "更新摘要",
                "category_id": 2,
                "tag_ids": [2],
                "status": "published",
            },
        ),
        200,
        "admin_update_post",
    )
    cap.call("admin_get_created_post", "GET", f"/api/admin/posts/_id/{post_id}", auth="admin")
    # Clear tag associations before the hard-delete scenario: GORM may have
    # created a post_tags FK at startup, and DeletePost ignores delete errors.
    expect(
        cap.call(
            "admin_update_post_clear_tags",
            "PUT",
            f"/api/admin/posts/_id/{post_id}",
            auth="admin",
            json_body={"tag_ids": []},
        ),
        200,
        "admin_update_post_clear_tags",
    )

    expect(
        cap.call(
            "admin_delete_post_soft",
            "DELETE",
            f"/api/admin/posts/_id/{post_id}",
            auth="admin",
        ),
        200,
        "admin_delete_post_soft",
    )
    cap.call("admin_get_soft_deleted_post", "GET", f"/api/admin/posts/_id/{post_id}", auth="admin")
    cap.call("search_after_soft_delete", "GET", q("/api/search", keyword="契约测试文章"))

    expect(
        cap.call(
            "admin_restore_post",
            "POST",
            f"/api/admin/posts/_id/{post_id}/restore",
            auth="admin",
        ),
        200,
        "admin_restore_post",
    )
    cap.call("admin_get_restored_post", "GET", f"/api/admin/posts/_id/{post_id}", auth="admin")
    cap.call("search_after_restore", "GET", q("/api/search", keyword="契约测试文章"))

    # Restore leaves status=draft, so a second DELETE soft-deletes again;
    # only a third DELETE on a hidden post takes the hard-delete branch.
    expect(
        cap.call(
            "admin_delete_post_soft_again",
            "DELETE",
            f"/api/admin/posts/_id/{post_id}",
            auth="admin",
        ),
        200,
        "admin_delete_post_soft_again",
    )
    cap.call(
        "admin_get_soft_deleted_post_again",
        "GET",
        f"/api/admin/posts/_id/{post_id}",
        auth="admin",
    )
    cap.call("search_after_soft_delete_again", "GET", q("/api/search", keyword="契约测试文章"))

    expect(
        cap.call(
            "admin_delete_post_hard",
            "DELETE",
            f"/api/admin/posts/_id/{post_id}",
            auth="admin",
        ),
        200,
        "admin_delete_post_hard",
    )
    cap.call("admin_get_hard_deleted_post", "GET", f"/api/admin/posts/_id/{post_id}", auth="admin")
    cap.call("search_after_hard_delete", "GET", q("/api/search", keyword="契约测试文章"))

    cap.call(
        "admin_delete_category",
        "DELETE",
        f"/api/admin/categories/_id/{category_id}",
        auth="admin",
    )
    cap.call("admin_delete_tag", "DELETE", f"/api/admin/tags/_id/{tag_id}", auth="admin")
    cap.call("admin_delete_page", "DELETE", f"/api/admin/pages/_id/{page_id}", auth="admin")
    cap.call(
        "admin_delete_draft_page",
        "DELETE",
        f"/api/admin/pages/_id/{draft_page_id}",
        auth="admin",
    )

    cap.call(
        "admin_approve_comment",
        "POST",
        "/api/admin/comments/_id/4/approve",
        auth="admin",
    )
    cap.call("comments_after_approve", "GET", q("/api/comments", post_id=1))
    cap.call("admin_reject_comment", "POST", "/api/admin/comments/_id/6/reject", auth="admin")
    cap.call("admin_delete_comment", "DELETE", "/api/admin/comments/_id/6", auth="admin")
    cap.call("admin_delete_missing_comment", "DELETE", "/api/admin/comments/_id/9999", auth="admin")

    cap.call("admin_delete_subscription", "DELETE", "/api/admin/subscriptions/_id/2", auth="admin")
    # Clear the subscription-confirmation email so the notify email is the
    # only message for newsub@example.com.
    mailhog.clear()
    notify = expect(
        cap.call("admin_notify", "POST", "/api/admin/notify", auth="admin", json_body={"post_id": 1}),
        200,
        "admin_notify",
    )
    cap.note("notify_response", notify)
    cap.email("email_notify_subscriber", "newsub@example.com")

    cap.call(
        "admin_update_user_role",
        "PUT",
        "/api/admin/users/2/role",
        auth="admin",
        json_body={"role": "admin"},
    )
    cap.call(
        "admin_update_user_role_missing",
        "PUT",
        "/api/admin/users/9999/role",
        auth="admin",
        json_body={"role": "admin"},
    )
    cap.call(
        "admin_update_user_role_bad_body",
        "PUT",
        "/api/admin/users/2/role",
        auth="admin",
        json_body={},
    )
    cap.call(
        "admin_update_user_role_revert",
        "PUT",
        "/api/admin/users/2/role",
        auth="admin",
        json_body={"role": "reader"},
    )

    cap.call("categories_after_delete", "GET", "/api/categories")
    cap.call("tags_after_delete", "GET", "/api/tags")
    cap.call("pages_after_delete", "GET", "/api/pages")
    cap.call("admin_posts_after_writes", "GET", "/api/admin/posts", auth="admin")
    cap.call("admin_comments_after_writes", "GET", "/api/admin/comments", auth="admin")
    cap.call("admin_subscriptions_after_writes", "GET", "/api/admin/subscriptions", auth="admin")

    cap.finish(meta)
    print(f"captured {cap.counter} fixtures into {out_dir}")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:  # noqa: BLE001
        print(f"contract capture failed: {exc}", file=sys.stderr)
        raise
