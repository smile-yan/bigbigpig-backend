#!/usr/bin/env python3
"""Smileyan 博客 API 命令行客户端（仅标准库）。

认证使用博客后台「令牌管理」创建的 API 令牌（sk_...），
通过环境变量 SMILEYAN_BLOG_TOKEN 读取；服务地址通过
SMILEYAN_BLOG_URL 读取（默认本机 http://127.0.0.1:8080）。

设计约束：
- 只接受 sk_ 令牌，不接受管理员 JWT，避免意外获得删除权限；
- 令牌只放在请求头里，不写入输出、不放进命令行参数；
- 所有成功结果输出为 JSON，错误走 stderr 并以非零码退出。
"""

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_URL = "http://127.0.0.1:8080"
TOKEN_ENV = "SMILEYAN_BLOG_TOKEN"
URL_ENV = "SMILEYAN_BLOG_URL"
DEFAULT_TIMEOUT = 30.0
USER_AGENT = "smileyan-blog-skill/1.0"

POST_STATUSES = ("draft", "published", "hidden")
COMMENT_STATUSES = ("pending", "approved", "rejected")
METHODS = ("GET", "POST", "PUT")

# 非交互 shell 通常不会加载 ~/.zshrc。环境变量缺失时，只读扫描这些文件里的
# SMILEYAN_BLOG_* 赋值行作为兜底；不执行文件，也不打印变量值。
ENV_FILES = (
    "~/.zshenv",
    "~/.zprofile",
    "~/.zshrc",
    "~/.bash_profile",
    "~/.bashrc",
    "~/.profile",
)
ENV_KEYS = ("SMILEYAN_BLOG_TOKEN", "SMILEYAN_BLOG_URL")


def load_shell_env():
    missing = [key for key in ENV_KEYS if not os.environ.get(key)]
    if not missing:
        return

    found = {}
    for pattern in ENV_FILES:
        path = os.path.expanduser(pattern)
        try:
            with open(path, "r", encoding="utf-8", errors="replace") as handle:
                for line in handle:
                    line = line.strip()
                    if not line or line.startswith("#"):
                        continue
                    match = re.match(
                        r"^(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$",
                        line,
                    )
                    if not match:
                        continue
                    name, raw = match.groups()
                    if name not in ENV_KEYS:
                        continue
                    raw = raw.strip()
                    if raw[:1] in ("'", '"'):
                        quote = raw[0]
                        end = raw.find(quote, 1)
                        if end == -1:
                            continue
                        value = raw[1:end]
                    else:
                        value = raw.split("#", 1)[0].strip()
                        if not value:
                            continue
                        value = value.split()[0]
                    found[name] = value
        except OSError:
            continue

    for key in missing:
        if found.get(key):
            os.environ[key] = found[key]


class UsageError(Exception):
    """本地参数/输入不合法；退出码 2。"""


class ConfigError(Exception):
    """缺少令牌或地址配置；退出码 2。"""


class ApiError(Exception):
    """服务端返回非 2xx；退出码 1。"""

    def __init__(self, status, body, url):
        self.status = status
        self.body = body
        self.url = url
        super().__init__(f"HTTP {status}")


class NetworkError(Exception):
    """连接失败/超时；退出码 1。"""


def json_text(value):
    return json.dumps(value, ensure_ascii=False, indent=2)


def decode_body(content_type, raw):
    text = raw.decode("utf-8", errors="replace")
    if "json" in (content_type or "").lower() or text.lstrip().startswith(("{", "[")):
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return text
    return text


class Client:
    def __init__(self, base_url, token, timeout=DEFAULT_TIMEOUT, verbose=False):
        self.base_url = (base_url or "").rstrip("/")
        if not self.base_url:
            raise ConfigError(f"{URL_ENV} 不能为空")
        self.token = (token or "").strip()
        self.timeout = timeout
        self.verbose = verbose

    def request(self, method, path, query=None, body=None, auth=False):
        if not path.startswith("/"):
            path = "/" + path

        url = self.base_url + path
        if query:
            pairs = [
                (key, value)
                for key, value in query.items()
                if value is not None and value != ""
            ]
            if pairs:
                url += "?" + urllib.parse.urlencode(pairs)

        headers = {
            "Accept": "application/json",
            "User-Agent": USER_AGENT,
        }
        if auth:
            self.require_token()
            headers["Authorization"] = "Bearer " + self.token

        data = None
        if body is not None:
            data = json.dumps(body, ensure_ascii=False).encode("utf-8")
            headers["Content-Type"] = "application/json"

        if self.verbose:
            print(f"{method} {url}", file=sys.stderr)

        request = urllib.request.Request(
            url, data=data, headers=headers, method=method
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                return decode_body(
                    response.headers.get("Content-Type", ""), response.read()
                )
        except urllib.error.HTTPError as error:
            content_type = ""
            if error.headers is not None:
                content_type = error.headers.get("Content-Type", "")
            raise ApiError(error.code, decode_body(content_type, error.read()), url)
        except TimeoutError:
            raise NetworkError(f"请求超时（{self.timeout:g}s）：{url}")
        except urllib.error.URLError as error:
            raise NetworkError(f"无法连接 {url}：{error.reason}")
        except OSError as error:
            raise NetworkError(f"网络请求失败：{error}")

    def require_token(self):
        if not self.token:
            raise ConfigError(
                f"缺少环境变量 {TOKEN_ENV}。请先在博客后台「令牌管理」创建 API 令牌，"
                f"再执行 export {TOKEN_ENV}=sk_..."
            )
        if not self.token.startswith("sk_"):
            raise ConfigError(
                f"{TOKEN_ENV} 必须是后台「令牌管理」创建的 sk_ 令牌"
                "（不要包含 Bearer 前缀）；本 skill 不接受管理员 JWT。"
            )


# ---------------------------------------------------------------------------
# 参数与输入辅助
# ---------------------------------------------------------------------------


def segment(value):
    return urllib.parse.quote(str(value), safe="")


def positive_int(value, label):
    try:
        number = int(value)
    except (TypeError, ValueError):
        raise UsageError(f"{label} 必须是整数，收到 {value!r}")
    if number <= 0:
        raise UsageError(f"{label} 必须大于 0，收到 {number}")
    return number


def positive_int_arg(value):
    try:
        number = int(value)
    except (TypeError, ValueError):
        raise argparse.ArgumentTypeError(f"需要整数，收到 {value!r}")
    if number <= 0:
        raise argparse.ArgumentTypeError(f"需要大于 0 的整数，收到 {number}")
    return number


def require_text(value, label):
    if value is None or value == "":
        raise UsageError(f"{label} 不能为空")
    return value


def parse_id_list(raw, label):
    values = []
    for piece in str(raw).split(","):
        piece = piece.strip()
        if not piece:
            continue
        values.append(positive_int(piece, label))
    if not values:
        raise UsageError(
            f"{label} 不能为空。更新文章时 tag_ids=[] 会把所有标签都挂上，"
            "因此不能用空列表清空标签；要保留原标签就省略该参数。"
        )
    return values


def content_from_args(args, required=False):
    if args.content is not None and args.content_file is not None:
        raise UsageError("--content 和 --content-file 只能提供一个")

    if args.content_file is not None:
        if args.content_file == "-":
            text = sys.stdin.read()
        else:
            try:
                with open(args.content_file, "r", encoding="utf-8") as handle:
                    text = handle.read()
            except OSError as error:
                raise UsageError(f"读取内容文件失败：{error}")
    elif args.content is not None:
        text = args.content
    else:
        if required:
            raise UsageError("必须提供 --content 或 --content-file")
        return None

    if required and text == "":
        raise UsageError("文章内容不能为空")
    if not required and text == "":
        raise UsageError(
            "内容为空时后端会保持原值；要保留原内容请省略 --content/--content-file"
        )
    return text


# ---------------------------------------------------------------------------
# 请求体构造
# ---------------------------------------------------------------------------


def post_body(args, create):
    body = {}

    if create:
        body["title"] = require_text(args.title, "--title")
        body["content"] = content_from_args(args, required=True)
    else:
        if args.title is not None:
            body["title"] = require_text(args.title, "--title")
        text = content_from_args(args, required=False)
        if text is not None:
            body["content"] = text

    if args.slug is not None:
        body["slug"] = require_text(args.slug, "--slug")
    if args.excerpt is not None:
        body["excerpt"] = require_text(args.excerpt, "--excerpt")
    if args.cover_image is not None:
        body["cover_image"] = require_text(args.cover_image, "--cover-image")
    if args.category_id is not None:
        body["category_id"] = positive_int(args.category_id, "--category-id")
    if args.tag_ids is not None:
        body["tag_ids"] = parse_id_list(args.tag_ids, "--tag-ids")
    if args.status is not None:
        body["status"] = args.status

    if not create and not body:
        raise UsageError("没有要更新的字段；至少提供标题、内容、状态等一个参数")
    return body


def page_body(args, create):
    body = {}

    if create:
        body["title"] = require_text(args.title, "--title")
    elif args.title is not None:
        body["title"] = require_text(args.title, "--title")

    text = content_from_args(args, required=False)
    if text is not None:
        body["content"] = text

    if args.slug is not None:
        body["slug"] = require_text(args.slug, "--slug")
    if args.status is not None:
        body["status"] = args.status

    if create:
        # 后端默认 published；CLI 选择更保守的草稿，避免误发布。
        body.setdefault("status", "draft")
    elif not body:
        raise UsageError("没有要更新的字段；至少提供标题、内容、状态等一个参数")
    return body


def tag_body(args, create):
    body = {}
    if args.name is not None:
        body["name"] = require_text(args.name, "--name")
    if args.slug is not None:
        body["slug"] = require_text(args.slug, "--slug")
    if create and "name" not in body:
        raise UsageError("创建标签必须提供 --name")
    if not create and not body:
        raise UsageError("没有要更新的字段；至少提供 --name 或 --slug")
    return body


def category_body(args, create):
    body = {}
    if args.name is not None:
        body["name"] = require_text(args.name, "--name")
    if args.slug is not None:
        body["slug"] = require_text(args.slug, "--slug")
    if args.description is not None:
        if create:
            body["description"] = args.description
        else:
            body["description"] = require_text(args.description, "--description")
    if args.sort is not None:
        body["sort"] = args.sort
        if not create and args.sort <= 0:
            print(
                "提示：后端只在 sort > 0 时更新排序，sort <= 0 会被忽略。",
                file=sys.stderr,
            )
    if create and "name" not in body:
        raise UsageError("创建分类必须提供 --name")
    if not create and not body:
        raise UsageError("没有要更新的字段；至少提供 --name、--slug、--description 或 --sort")
    return body


# ---------------------------------------------------------------------------
# 命令实现
# ---------------------------------------------------------------------------


def cmd_health(client, args):
    return client.request("GET", "/api/hello")


def cmd_check(client, args):
    data = client.request(
        "GET", "/api/admin/posts/summary", query={"page_size": 1}, auth=True
    )
    return {
        "ok": True,
        "url": client.base_url,
        "total_posts": data.get("total") if isinstance(data, dict) else None,
    }


def cmd_posts_list(client, args):
    path = "/api/admin/posts" if args.full else "/api/admin/posts/summary"
    query = {
        "page": args.page,
        "page_size": args.page_size,
        "status": args.status,
        "category_id": args.category_id,
        "tag_id": args.tag_id,
        "keyword": args.keyword,
        "start_date": args.start_date,
        "end_date": args.end_date,
    }
    return client.request("GET", path, query=query, auth=True)


def cmd_posts_get(client, args):
    ref = str(args.ref)
    if args.slug or not ref.isdigit():
        path = "/api/admin/posts/slug/" + segment(ref)
    else:
        path = "/api/admin/posts/_id/" + segment(ref)
    return client.request("GET", path, auth=True)


def cmd_posts_create(client, args):
    body = post_body(args, create=True)
    return client.request("POST", "/api/admin/posts", body=body, auth=True)


def cmd_posts_update(client, args):
    post_id = positive_int(args.id, "文章 ID")
    body = post_body(args, create=False)
    return client.request(
        "PUT", "/api/admin/posts/_id/" + segment(post_id), body=body, auth=True
    )


def cmd_pages_list(client, args):
    return client.request(
        "GET",
        "/api/admin/pages",
        query={"page": args.page, "page_size": args.page_size},
        auth=True,
    )


def cmd_pages_get(client, args):
    ref = str(args.ref)
    if not args.slug and ref.isdigit():
        return client.request(
            "GET", "/api/admin/pages/_id/" + segment(ref), auth=True
        )

    # 公共路由对 sk_ 令牌等同匿名，只能看到 published；草稿需要扫后台列表。
    try:
        return client.request("GET", "/api/pages/" + segment(ref))
    except ApiError as error:
        if error.status != 404:
            raise

    listing = client.request(
        "GET",
        "/api/admin/pages",
        query={"page": 1, "page_size": 1000},
        auth=True,
    )
    for item in listing.get("list", []) if isinstance(listing, dict) else []:
        if item.get("slug") == ref:
            return item
    raise ApiError(404, {"error": "Page not found"}, "/api/admin/pages")


def cmd_pages_create(client, args):
    body = page_body(args, create=True)
    result = client.request("POST", "/api/admin/pages", body=body, auth=True)
    if body.get("status") == "draft":
        print("提示：页面已保存为草稿，未公开发布。", file=sys.stderr)
    return result


def cmd_pages_update(client, args):
    page_id = positive_int(args.id, "页面 ID")
    body = page_body(args, create=False)
    return client.request(
        "PUT", "/api/admin/pages/_id/" + segment(page_id), body=body, auth=True
    )


def cmd_tags_list(client, args):
    return client.request("GET", "/api/tags")


def cmd_tags_create(client, args):
    body = tag_body(args, create=True)
    return client.request("POST", "/api/admin/tags", body=body, auth=True)


def cmd_tags_update(client, args):
    tag_id = positive_int(args.id, "标签 ID")
    body = tag_body(args, create=False)
    return client.request(
        "PUT", "/api/admin/tags/_id/" + segment(tag_id), body=body, auth=True
    )


def cmd_categories_list(client, args):
    return client.request("GET", "/api/categories")


def cmd_categories_create(client, args):
    body = category_body(args, create=True)
    return client.request("POST", "/api/admin/categories", body=body, auth=True)


def cmd_categories_update(client, args):
    category_id = positive_int(args.id, "分类 ID")
    body = category_body(args, create=False)
    return client.request(
        "PUT",
        "/api/admin/categories/_id/" + segment(category_id),
        body=body,
        auth=True,
    )


def cmd_comments_list(client, args):
    return client.request(
        "GET",
        "/api/admin/comments",
        query={
            "page": args.page,
            "page_size": args.page_size,
            "status": args.status,
        },
        auth=True,
    )


def cmd_comments_public(client, args):
    return client.request(
        "GET", "/api/comments", query={"post_id": args.post_id}
    )


def cmd_comments_approve(client, args):
    comment_id = positive_int(args.id, "评论 ID")
    return client.request(
        "POST",
        "/api/admin/comments/_id/" + segment(comment_id) + "/approve",
        body={},
        auth=True,
    )


def cmd_comments_reject(client, args):
    comment_id = positive_int(args.id, "评论 ID")
    return client.request(
        "POST",
        "/api/admin/comments/_id/" + segment(comment_id) + "/reject",
        body={},
        auth=True,
    )


def cmd_search(client, args):
    keyword = require_text(args.keyword, "关键词")
    return client.request(
        "GET",
        "/api/search",
        query={
            "keyword": keyword,
            "page": args.page,
            "page_size": args.page_size,
        },
    )


def cmd_dates(client, args):
    return client.request("GET", "/api/posts/dates")


def cmd_read(client, args):
    slug = require_text(args.slug, "文章 slug")
    query = {"edit": "true"} if args.no_view_count else None
    return client.request("GET", "/api/posts/" + segment(slug), query=query)


def cmd_api(client, args):
    method = args.method.upper()
    if method not in METHODS:
        raise UsageError(
            "通用请求只允许 GET/POST/PUT；API 令牌不能删除内容，"
            "需要删除请在博客后台用管理员账号操作。"
        )
    path = args.path
    if not path.startswith("/"):
        raise UsageError("PATH 必须以 / 开头，例如 /api/admin/posts/summary")

    body = None
    if method == "GET":
        if args.data is not None or args.data_file is not None:
            raise UsageError("GET 请求不接受 --data/--data-file")
    else:
        if args.data is not None and args.data_file is not None:
            raise UsageError("--data 和 --data-file 只能提供一个")
        raw = None
        if args.data_file is not None:
            try:
                with open(args.data_file, "r", encoding="utf-8") as handle:
                    raw = handle.read()
            except OSError as error:
                raise UsageError(f"读取 JSON 文件失败：{error}")
        elif args.data is not None:
            raw = args.data
        if raw is not None:
            try:
                body = json.loads(raw)
            except json.JSONDecodeError as error:
                raise UsageError(f"--data 不是合法 JSON：{error}")

    return client.request(method, path, body=body, auth=True)


# ---------------------------------------------------------------------------
# 命令行解析
# ---------------------------------------------------------------------------


def add_common_options(parser):
    parser.add_argument(
        "--url",
        default=argparse.SUPPRESS,
        help=f"博客地址，默认 ${URL_ENV} 或 {DEFAULT_URL}",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=argparse.SUPPRESS,
        help=f"请求超时秒数，默认 {DEFAULT_TIMEOUT:g}",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        default=argparse.SUPPRESS,
        help="把请求方法和 URL 打印到 stderr（不包含令牌）",
    )


def add_source_options(parser, content_help):
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--content", help=content_help)
    group.add_argument("--content-file", help="从 UTF-8 文件读取内容；- 表示 stdin")


def add_post_options(parser, create):
    if create:
        parser.add_argument("--title", required=True, help="文章标题（必填）")
    else:
        parser.add_argument("--title", help="新标题；不传则保持不变")
    add_source_options(parser, "Markdown 正文" + ("（必填）" if create else "；不传则保持不变"))
    parser.add_argument("--slug", help="自定义 slug；默认由标题生成，重复会 400")
    parser.add_argument("--excerpt", help="摘要；不传则保持不变")
    parser.add_argument("--cover-image", help="封面图片 URL；不传则保持不变")
    parser.add_argument("--category-id", type=int, help="分类 ID")
    parser.add_argument(
        "--tag-ids",
        help="逗号分隔的标签 ID，如 1,3,5；更新时传完整列表，空列表会被拒绝",
    )
    parser.add_argument(
        "--status",
        choices=POST_STATUSES,
        help="文章状态；创建时省略等于 draft",
    )


def add_page_options(parser, create):
    if create:
        parser.add_argument("--title", required=True, help="页面标题（必填）")
    else:
        parser.add_argument("--title", help="新标题；不传则保持不变")
    add_source_options(parser, "Markdown 正文" + ("；不传则为空" if create else "；不传则保持不变"))
    parser.add_argument("--slug", help="自定义 slug；默认由标题生成")
    parser.add_argument(
        "--status",
        choices=POST_STATUSES,
        help="页面状态；CLI 创建时省略等于 draft（比后端默认更保守）",
    )


def build_parser():
    parser = argparse.ArgumentParser(
        prog="blog.py",
        description=(
            "Smileyan 博客管理 CLI。使用后台「令牌管理」创建的 API 令牌，"
            f"从环境变量 {TOKEN_ENV} 读取；地址从 {URL_ENV} 读取。"
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "示例：\n"
            "  export SMILEYAN_BLOG_TOKEN=sk_...\n"
            "  python3 blog.py check\n"
            "  python3 blog.py posts list --keyword 博客 --page-size 20\n"
            "  python3 blog.py posts create --title '新文章' --content-file post.md\n"
            "  python3 blog.py posts update 42 --status published\n"
            "  python3 blog.py comments list --status pending\n"
            "  python3 blog.py comments approve 7\n"
            "\n令牌不能删除内容；删除请在博客后台用管理员账号完成。\n"
        ),
    )
    add_common_options(parser)
    subparsers = parser.add_subparsers(dest="command", required=True)

    p = subparsers.add_parser("health", help="公共健康检查 GET /api/hello")
    add_common_options(p)
    p.set_defaults(func=cmd_health)

    p = subparsers.add_parser(
        "check", help="验证令牌是否可用（读取一页后台文章）"
    )
    add_common_options(p)
    p.set_defaults(func=cmd_check)

    p = subparsers.add_parser("dates", help="文章发布日历 GET /api/posts/dates")
    add_common_options(p)
    p.set_defaults(func=cmd_dates)

    # posts
    posts = subparsers.add_parser("posts", help="文章管理")
    posts_sub = posts.add_subparsers(dest="action", required=True)

    p = posts_sub.add_parser("list", help="后台文章列表（默认轻量 summary）")
    add_common_options(p)
    p.add_argument("--page", type=positive_int_arg)
    p.add_argument("--page-size", type=positive_int_arg)
    p.add_argument("--status", choices=POST_STATUSES)
    p.add_argument("--category-id", type=positive_int_arg)
    p.add_argument("--tag-id", type=positive_int_arg)
    p.add_argument("--keyword", help="标题关键词（LIKE 匹配）")
    p.add_argument("--start-date", help="起始日期 YYYY-MM-DD（含）")
    p.add_argument("--end-date", help="结束日期 YYYY-MM-DD（含）")
    p.add_argument(
        "--full",
        action="store_true",
        help="使用 GET /api/admin/posts，返回完整正文（载荷更大）",
    )
    p.set_defaults(func=cmd_posts_list)

    p = posts_sub.add_parser("get", help="按 ID 或 slug 读取后台文章详情")
    add_common_options(p)
    p.add_argument("ref", help="数字 ID 或 slug")
    p.add_argument("--slug", action="store_true", help="强制按 slug 查询")
    p.set_defaults(func=cmd_posts_get)

    p = posts_sub.add_parser("create", help="创建文章（默认草稿）")
    add_common_options(p)
    add_post_options(p, create=True)
    p.set_defaults(func=cmd_posts_create)

    p = posts_sub.add_parser("update", help="更新文章；只传要改的字段")
    add_common_options(p)
    p.add_argument("id", help="文章 ID")
    add_post_options(p, create=False)
    p.set_defaults(func=cmd_posts_update)

    # pages
    pages = subparsers.add_parser("pages", help="页面管理")
    pages_sub = pages.add_subparsers(dest="action", required=True)

    p = pages_sub.add_parser("list", help="后台页面列表（含草稿和正文）")
    add_common_options(p)
    p.add_argument("--page", type=positive_int_arg)
    p.add_argument("--page-size", type=positive_int_arg)
    p.set_defaults(func=cmd_pages_list)

    p = pages_sub.add_parser("get", help="按 ID 或 slug 读取页面")
    add_common_options(p)
    p.add_argument("ref", help="数字 ID 或 slug")
    p.add_argument("--slug", action="store_true", help="强制按 slug 查询")
    p.set_defaults(func=cmd_pages_get)

    p = pages_sub.add_parser("create", help="创建页面（CLI 默认 draft）")
    add_common_options(p)
    add_page_options(p, create=True)
    p.set_defaults(func=cmd_pages_create)

    p = pages_sub.add_parser("update", help="更新页面；只传要改的字段")
    add_common_options(p)
    p.add_argument("id", help="页面 ID")
    add_page_options(p, create=False)
    p.set_defaults(func=cmd_pages_update)

    # tags
    tags = subparsers.add_parser("tags", help="标签管理")
    tags_sub = tags.add_subparsers(dest="action", required=True)

    p = tags_sub.add_parser("list", help="公共标签列表（含 post_count）")
    add_common_options(p)
    p.set_defaults(func=cmd_tags_list)

    p = tags_sub.add_parser("create", help="创建标签")
    add_common_options(p)
    p.add_argument("--name", required=True)
    p.add_argument("--slug")
    p.set_defaults(func=cmd_tags_create)

    p = tags_sub.add_parser("update", help="更新标签名称或 slug")
    add_common_options(p)
    p.add_argument("id", help="标签 ID")
    p.add_argument("--name")
    p.add_argument("--slug")
    p.set_defaults(func=cmd_tags_update)

    # categories
    categories = subparsers.add_parser("categories", help="分类管理")
    categories_sub = categories.add_subparsers(dest="action", required=True)

    p = categories_sub.add_parser("list", help="公共分类列表（含 post_count）")
    add_common_options(p)
    p.set_defaults(func=cmd_categories_list)

    p = categories_sub.add_parser("create", help="创建分类")
    add_common_options(p)
    p.add_argument("--name", required=True)
    p.add_argument("--slug")
    p.add_argument("--description")
    p.add_argument("--sort", type=int)
    p.set_defaults(func=cmd_categories_create)

    p = categories_sub.add_parser("update", help="更新分类")
    add_common_options(p)
    p.add_argument("id", help="分类 ID")
    p.add_argument("--name")
    p.add_argument("--slug")
    p.add_argument("--description")
    p.add_argument("--sort", type=int, help="后端只应用 > 0 的值")
    p.set_defaults(func=cmd_categories_update)

    # comments
    comments = subparsers.add_parser("comments", help="评论读取与审核")
    comments_sub = comments.add_subparsers(dest="action", required=True)

    p = comments_sub.add_parser("list", help="后台评论列表")
    add_common_options(p)
    p.add_argument("--page", type=positive_int_arg)
    p.add_argument("--page-size", type=positive_int_arg)
    p.add_argument("--status", choices=COMMENT_STATUSES)
    p.set_defaults(func=cmd_comments_list)

    p = comments_sub.add_parser("public", help="公共评论列表（只看已通过）")
    add_common_options(p)
    p.add_argument("--post-id", type=positive_int_arg, required=True)
    p.set_defaults(func=cmd_comments_public)

    p = comments_sub.add_parser("approve", help="通过评论审核")
    add_common_options(p)
    p.add_argument("id", help="评论 ID")
    p.set_defaults(func=cmd_comments_approve)

    p = comments_sub.add_parser("reject", help="拒绝评论审核")
    add_common_options(p)
    p.add_argument("id", help="评论 ID")
    p.set_defaults(func=cmd_comments_reject)

    # search / read
    p = subparsers.add_parser("search", help="公共搜索 GET /api/search")
    add_common_options(p)
    p.add_argument("keyword")
    p.add_argument("--page", type=positive_int_arg)
    p.add_argument("--page-size", type=positive_int_arg)
    p.set_defaults(func=cmd_search)

    p = subparsers.add_parser("read", help="按 slug 读取已发布文章")
    add_common_options(p)
    p.add_argument("slug")
    p.add_argument(
        "--no-view-count",
        action="store_true",
        help="传 edit=true，不增加浏览数",
    )
    p.set_defaults(func=cmd_read)

    # generic
    p = subparsers.add_parser(
        "api", help="带令牌的通用请求（仅 GET/POST/PUT）"
    )
    add_common_options(p)
    p.add_argument("method", help="GET、POST 或 PUT")
    p.add_argument("path", help="以 / 开头的路径，可自带查询串")
    p.add_argument("--data", help="JSON 请求体")
    p.add_argument("--data-file", help="从 UTF-8 文件读取 JSON 请求体")
    p.set_defaults(func=cmd_api)

    return parser


def print_api_error(error):
    body = error.body
    if isinstance(body, (dict, list)):
        text = json.dumps(body, ensure_ascii=False)
    else:
        text = str(body).strip()
    print(f"HTTP {error.status}: {text}", file=sys.stderr)

    if error.status == 401:
        print(
            "提示：令牌无效、已过期或被撤销。请让用户在前端后台「令牌管理」"
            f"重新创建，并更新 {TOKEN_ENV}。",
            file=sys.stderr,
        )
    elif error.status == 403:
        print(
            "提示：该操作需要管理员 JWT；API 令牌没有这个权限。",
            file=sys.stderr,
        )
    elif error.status == 400:
        print(
            "提示：请求字段不合法（常见原因：标题/正文为空、slug 重复、状态值不支持）。",
            file=sys.stderr,
        )
    elif error.status == 404:
        print(
            "提示：资源不存在。公共接口只返回已发布内容；草稿请用 posts get 或 pages get。",
            file=sys.stderr,
        )
    elif error.status == 413:
        print(
            "提示：请求体超过后端 2 MiB 上限，请压缩或拆分内容。",
            file=sys.stderr,
        )


def main(argv=None):
    load_shell_env()
    parser = build_parser()

    try:
        args = parser.parse_args(argv)
        client = Client(
            getattr(args, "url", os.environ.get(URL_ENV, DEFAULT_URL)),
            os.environ.get(TOKEN_ENV, ""),
            getattr(args, "timeout", DEFAULT_TIMEOUT),
            getattr(args, "verbose", False),
        )
        result = args.func(client, args)
        if result is not None:
            print(json_text(result))
        return 0
    except UsageError as error:
        print(f"参数错误：{error}", file=sys.stderr)
        return 2
    except ConfigError as error:
        print(f"配置错误：{error}", file=sys.stderr)
        return 2
    except ApiError as error:
        print_api_error(error)
        return 1
    except NetworkError as error:
        print(f"网络错误：{error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
