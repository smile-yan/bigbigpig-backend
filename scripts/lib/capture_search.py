#!/usr/bin/env python3
"""Capture search results for the Phase 0 corpus.

Writes one JSON file per query plus an ordered index. The results are the
acceptance fixture for the Rust/tantivy implementation (top-10 overlap, total).
"""
from __future__ import annotations

import json
import pathlib
import sys
import urllib.parse

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from capture_common import (  # noqa: E402
    CAPTURE_CLIENT_IP,
    env,
    file_sha256,
    http_request,
    normalize,
)


def main() -> None:
    base_url = env("CAPTURE_BASE_URL", "http://127.0.0.1:18080").rstrip("/")
    queries_path = pathlib.Path(env("CAPTURE_QUERIES", "tests/fixtures/search/queries.json"))
    out_dir = pathlib.Path(env("CAPTURE_SEARCH_OUT_DIR", "tests/fixtures/search/results"))

    doc = json.loads(queries_path.read_text())
    queries = doc["queries"]

    if out_dir.exists():
        for path in out_dir.glob("*.json"):
            path.unlink()
    out_dir.mkdir(parents=True, exist_ok=True)

    index = []
    skipped = []
    for query in queries:
        params = {
            "keyword": query["keyword"],
            "page": query.get("page", 1),
            "page_size": query.get("page_size", 10),
        }
        url = f"{base_url}/api/search?{urllib.parse.urlencode(params)}"
        status, _, body = http_request(
            "GET",
            url,
            headers={
                "User-Agent": "smileyan-capture/1.0",
                "X-Forwarded-For": CAPTURE_CLIENT_IP,
                "Accept": "*/*",
            },
        )
        try:
            parsed = json.loads(body)
        except json.JSONDecodeError:
            parsed = None

        normalized = normalize(parsed) if parsed is not None else None
        items = normalized.get("list", []) if isinstance(normalized, dict) else []
        slugs = [item.get("slug") for item in items] if isinstance(items, list) else []
        record = {
            "id": query["id"],
            "kind": query["kind"],
            "keyword": query["keyword"],
            "page": params["page"],
            "page_size": params["page_size"],
            "status": status,
            "total": normalized.get("total") if isinstance(normalized, dict) else None,
            "list_length": len(items) if isinstance(items, list) else 0,
            "slugs": slugs,
            "body": normalized,
        }
        filename = f"{query['id']}.json"
        (out_dir / filename).write_text(
            json.dumps(record, ensure_ascii=False, indent=2) + "\n"
        )
        index.append(
            {
                "id": query["id"],
                "kind": query["kind"],
                "file": filename,
                "status": status,
                "total": record["total"],
                "list_length": record["list_length"],
            }
        )
        if status != 200:
            skipped.append(query["id"])

    meta = {
        "backend_commit": env("CAPTURE_BACKEND_COMMIT", "unknown"),
        "backend_tag": env("CAPTURE_BACKEND_TAG", ""),
        "queries_sha256": file_sha256(queries_path),
        "query_count": len(queries),
        "non_200": skipped,
    }
    (out_dir.parent / "results_index.json").write_text(
        json.dumps({"meta": meta, "results": index}, ensure_ascii=False, indent=2) + "\n"
    )
    print(f"captured {len(index)} search results into {out_dir}")
    if skipped:
        print(f"non-200 queries: {', '.join(skipped)}")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:  # noqa: BLE001
        print(f"search capture failed: {exc}", file=sys.stderr)
        raise
