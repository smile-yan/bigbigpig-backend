#!/usr/bin/env python3
"""Shared helpers for the Phase 0 capture harness (stdlib only)."""
from __future__ import annotations

import base64
import email.header
import hashlib
import hmac
import json
import os
import pathlib
import quopri
import re
import shutil
import socket
import time
import urllib.error
import urllib.parse
import urllib.request

# A deterministic client IP: sent as X-Forwarded-For so Redis rate-limit keys
# are stable across runs and the Go/Gin ClientIP behaviour is exercised.
CAPTURE_CLIENT_IP = "203.0.113.10"

NOT_SET = object()

KEEP_RESPONSE_HEADERS = {
    "access-control-allow-headers",
    "access-control-allow-methods",
    "access-control-allow-origin",
    "access-control-expose-headers",
    "cache-control",
    "connection",
    "content-disposition",
    "content-type",
    "location",
    "transfer-encoding",
}

TIME_KEYS = {"created_at", "updated_at", "deleted_at"}
HEX32 = re.compile(r"^[0-9a-f]{32}$")
HEX8 = re.compile(r"^[0-9a-f]{8}$")
JWT = re.compile(r"^eyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$")
SIX_DIGITS = re.compile(r"(?<!\d)\d{6}(?!\d)")


def env(name: str, default: str = "") -> str:
    return os.environ.get(name, default)


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_sha256(path: pathlib.Path) -> str:
    return sha256_hex(path.read_bytes())


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Return 3xx as-is instead of following; Gin's trailing-slash 301 matters."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


_OPENER = urllib.request.build_opener(_NoRedirect)


def http_request(
    method: str,
    url: str,
    headers: dict | None = None,
    body: bytes | None = None,
    timeout: int = 30,
) -> tuple[int, dict, bytes]:
    req = urllib.request.Request(url, data=body, headers=headers or {}, method=method)
    try:
        with _OPENER.open(req, timeout=timeout) as resp:
            return resp.status, {k.lower(): v for k, v in resp.headers.items()}, resp.read()
    except urllib.error.HTTPError as exc:
        return exc.code, {k.lower(): v for k, v in exc.headers.items()}, exc.read()


def wait_http(url: str, timeout: float = 60.0) -> None:
    deadline = time.time() + timeout
    last_error = None
    while time.time() < deadline:
        try:
            status, _, _ = http_request("GET", url, timeout=2)
            if status == 200:
                return
            last_error = RuntimeError(f"HTTP {status}")
        except (urllib.error.URLError, OSError) as exc:
            last_error = exc
        time.sleep(0.5)
    raise RuntimeError(f"timed out waiting for {url}: {last_error}")


def b64url(raw: bytes) -> bytes:
    return base64.urlsafe_b64encode(raw).rstrip(b"=")


def make_jwt(payload: dict, secret: str, alg: str = "HS256") -> str:
    """Sign a JWT with the same field names the Go baseline uses."""
    header = {"alg": alg, "typ": "JWT"}
    signing_input = b".".join(
        [
            b64url(json.dumps(header, separators=(",", ":")).encode()),
            b64url(json.dumps(payload, separators=(",", ":"), ensure_ascii=False).encode()),
        ]
    )
    if alg == "HS256":
        digest = hmac.new(secret.encode(), signing_input, hashlib.sha256).digest()
    elif alg == "HS512":
        digest = hmac.new(secret.encode(), signing_input, hashlib.sha512).digest()
    else:
        raise ValueError(f"unsupported alg {alg}")
    return (signing_input + b"." + b64url(digest)).decode()


def normalize(value, key: str | None = None):
    """Normalize volatile values so repeated captures are identical."""
    if isinstance(value, dict):
        return {k: normalize(v, k) for k, v in value.items()}
    if isinstance(value, list):
        return [normalize(v, key) for v in value]
    if isinstance(value, str):
        if key in TIME_KEYS and value:
            return "<timestamp>"
        if JWT.match(value):
            return "<jwt>"
        if HEX32.match(value):
            return "<token>"
        if key == "slug" and HEX8.match(value):
            return "<random-slug>"
    return value


def normalize_request(value):
    """Redact volatile request secrets before writing the fixture."""
    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            if key in {"code", "token", "password", "api_key"} and isinstance(item, str) and item:
                out[key] = f"<{key}>"
            else:
                out[key] = normalize_request(item)
        return out
    if isinstance(value, list):
        return [normalize_request(item) for item in value]
    return value


def normalize_email_text(text: str) -> str:
    text = JWT.sub("<jwt>", text)
    text = re.sub(r"\b[0-9a-f]{32}\b", "<token>", text)
    text = SIX_DIGITS.sub("<code>", text)
    return text


def decode_mime(value) -> str:
    if isinstance(value, list):
        value = value[0] if value else ""
    if not isinstance(value, str):
        return ""
    parts = email.header.decode_header(value)
    out = []
    for part, charset in parts:
        if isinstance(part, bytes):
            out.append(part.decode(charset or "utf-8", "replace"))
        else:
            out.append(part)
    return "".join(out)


class MailHog:
    def __init__(self, base_url: str):
        self.base_url = base_url.rstrip("/")

    def clear(self) -> None:
        http_request("DELETE", f"{self.base_url}/api/v1/messages")

    def messages(self) -> list[dict]:
        status, _, body = http_request("GET", f"{self.base_url}/api/v2/messages?limit=200")
        if status != 200:
            raise RuntimeError(f"MailHog returned HTTP {status}")
        return json.loads(body).get("items", [])

    def latest(self, to: str) -> dict:
        matches = []
        for item in self.messages():
            recipients = []
            for recipient in item.get("To") or []:
                mailbox = recipient.get("Mailbox", "")
                domain = recipient.get("Domain", "")
                recipients.append(f"{mailbox}@{domain}")
            if to in recipients:
                matches.append(item)
        if not matches:
            raise RuntimeError(f"no MailHog message for {to}")
        matches.sort(key=lambda item: item.get("Created", ""))
        return matches[-1]

    def wait_for(self, to: str, timeout: float = 15.0) -> dict:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                return self.latest(to)
            except RuntimeError:
                time.sleep(0.5)
        raise RuntimeError(f"timed out waiting for MailHog message to {to}")

    @staticmethod
    def parsed(message: dict) -> dict:
        content = message.get("Content") or {}
        headers = content.get("Headers") or {}
        body = content.get("Body", "")
        transfer_encoding = decode_mime(headers.get("Content-Transfer-Encoding")).lower()
        # MailHog returns the raw MIME body; decode it so fixtures contain the
        # HTML the user actually sees (and token normalization is reliable).
        if transfer_encoding == "quoted-printable" or (
            not transfer_encoding and re.search(r"=[0-9A-Fa-f]{2}", body)
        ):
            body = quopri.decodestring(body.encode("utf-8")).decode("utf-8", "replace")
        elif transfer_encoding == "base64":
            try:
                body = base64.b64decode(body).decode("utf-8", "replace")
            except (ValueError, UnicodeDecodeError):
                pass
        return {
            "to": [
                f"{r.get('Mailbox', '')}@{r.get('Domain', '')}"
                for r in (message.get("To") or [])
            ],
            "from": (
                f"{(message.get('From') or {}).get('Mailbox', '')}@"
                f"{(message.get('From') or {}).get('Domain', '')}"
            ),
            "subject": decode_mime(headers.get("Subject")),
            "body": body,
        }

    @staticmethod
    def code(message: dict) -> str:
        parsed = MailHog.parsed(message)
        for text in (parsed["subject"], parsed["body"]):
            match = SIX_DIGITS.search(text)
            if match:
                return match.group(0)
        raise RuntimeError("no 6-digit verification code in message")


class RedisClient:
    """Minimal RESP2 client used only to snapshot rate-limit state."""

    def __init__(self, host: str, port: int, password: str = "", db: int = 0):
        self.host = host
        self.port = port
        self.password = password
        self.db = db

    def cmd(self, *args):
        sock = socket.create_connection((self.host, self.port), timeout=5)
        try:
            if self.password:
                self._send(sock, "AUTH", self.password)
            if self.db:
                self._send(sock, "SELECT", str(self.db))
            return self._send(sock, *args)
        finally:
            sock.close()

    def _send(self, sock: socket.socket, *args):
        payload = [f"*{len(args)}\r\n".encode()]
        for arg in args:
            data = str(arg).encode()
            payload.append(f"${len(data)}\r\n".encode() + data + b"\r\n")
        sock.sendall(b"".join(payload))
        return self._read(sock)

    def _read(self, sock: socket.socket):
        line = self._readline(sock)
        prefix, rest = line[:1], line[1:]
        if prefix == b"+":
            return rest.decode()
        if prefix == b"-":
            raise RuntimeError(f"redis error: {rest.decode()}")
        if prefix == b":":
            return int(rest)
        if prefix == b"$":
            length = int(rest)
            if length == -1:
                return None
            data = self._read_exact(sock, length)
            self._read_exact(sock, 2)
            return data.decode("utf-8", "replace")
        if prefix == b"*":
            length = int(rest)
            if length == -1:
                return None
            return [self._read(sock) for _ in range(length)]
        raise RuntimeError(f"unexpected RESP prefix {prefix!r}")

    @staticmethod
    def _readline(sock: socket.socket) -> bytes:
        buf = bytearray()
        while not buf.endswith(b"\r\n"):
            chunk = sock.recv(1)
            if not chunk:
                raise RuntimeError("redis connection closed")
            buf.extend(chunk)
        return bytes(buf[:-2])

    @staticmethod
    def _read_exact(sock: socket.socket, n: int) -> bytes:
        buf = bytearray()
        while len(buf) < n:
            chunk = sock.recv(n - len(buf))
            if not chunk:
                raise RuntimeError("redis connection closed")
            buf.extend(chunk)
        return bytes(buf)


def ttl_class(ttl: int) -> str:
    if ttl == -2:
        return "missing"
    if ttl == -1:
        return "no-expiry"
    if ttl <= 600:
        return "<=10m"
    if ttl <= 3600:
        return "<=1h"
    return ">1h"


def redis_snapshot(redis: RedisClient, email: str) -> dict:
    verification_key = f"verification_code:{email}"
    email_key = f"rate_limit:email:{email}"
    ip_keys = sorted(redis.cmd("KEYS", "rate_limit:ip:*") or [])
    return {
        "verification_code": {
            "exists": redis.cmd("EXISTS", verification_key) == 1,
            "ttl_class": ttl_class(redis.cmd("TTL", verification_key)),
        },
        "rate_limit_email": {
            "exists": redis.cmd("EXISTS", email_key) == 1,
            "ttl_class": ttl_class(redis.cmd("TTL", email_key)),
        },
        "rate_limit_ip": {
            key: {
                "value": redis.cmd("GET", key),
                "ttl_class": ttl_class(redis.cmd("TTL", key)),
            }
            for key in ip_keys
        },
    }


class Capture:
    """Writes one JSON file per request/email/note plus an ordered index."""

    def __init__(
        self,
        out_dir: pathlib.Path,
        base_url: str,
        tokens: dict[str, str],
        mailhog: MailHog | None = None,
        redis: RedisClient | None = None,
    ):
        self.out_dir = out_dir
        out_dir.mkdir(parents=True, exist_ok=True)
        self.case_dir = out_dir / "cases"
        self.email_dir = out_dir / "emails"
        self.note_dir = out_dir / "notes"
        for directory in (self.case_dir, self.email_dir, self.note_dir):
            if directory.exists():
                shutil.rmtree(directory)
            directory.mkdir()
        index_path = out_dir / "index.json"
        if index_path.exists():
            index_path.unlink()
        self.base_url = base_url.rstrip("/")
        self.tokens = tokens
        self.mailhog = mailhog
        self.redis = redis
        self.counter = 0
        self.index: list[dict] = []

    def _name(self, name: str, suffix: str) -> str:
        self.counter += 1
        return f"{self.counter:03d}_{name}{suffix}"

    def call(
        self,
        name: str,
        method: str,
        path: str,
        *,
        auth: str | None = None,
        json_body=NOT_SET,
        raw_body: bytes | None = None,
        content_type: str | None = None,
        extra_headers: dict | None = None,
        timeout: int = 30,
        multipart: dict | None = None,
    ):
        headers = {
            "User-Agent": "smileyan-capture/1.0",
            "X-Forwarded-For": CAPTURE_CLIENT_IP,
            "Accept": "*/*",
        }
        if extra_headers:
            headers.update(extra_headers)
        if auth:
            headers["Authorization"] = f"Bearer {self.tokens[auth]}"

        body = None
        if json_body is not NOT_SET:
            body = json.dumps(json_body, ensure_ascii=False).encode()
            headers["Content-Type"] = "application/json"
        elif raw_body is not None:
            body = raw_body
            if content_type:
                headers["Content-Type"] = content_type

        url = self.base_url + path
        status, response_headers, response_body = http_request(
            method, url, headers=headers, body=body, timeout=timeout
        )

        response: dict = {
            "status": status,
            "headers": {
                key: value
                for key, value in sorted(response_headers.items())
                if key in KEEP_RESPONSE_HEADERS
            },
        }
        parsed = None
        content_type_header = response_headers.get("content-type", "")
        if not response_body:
            response["empty"] = True
        elif "json" in content_type_header:
            try:
                parsed = json.loads(response_body)
                response["json"] = normalize(parsed)
            except json.JSONDecodeError:
                response["text"] = response_body.decode("utf-8", "replace")
        elif content_type_header.startswith("text/") or "javascript" in content_type_header:
            response["text"] = response_body.decode("utf-8", "replace")
        else:
            response["body_base64"] = base64.b64encode(response_body).decode()
            response["body_sha256"] = sha256_hex(response_body)
            response["body_bytes"] = len(response_body)

        request: dict = {
            "method": method,
            "path": path,
            "headers": {
                key: headers[key]
                for key in sorted(headers)
                if key in {"Authorization", "Content-Type", "X-Forwarded-For"}
            },
        }
        if auth:
            request["headers"]["Authorization"] = f"Bearer <{auth}>"
        if json_body is not NOT_SET:
            request["json"] = normalize_request(json_body)
        elif multipart is not None:
            request["multipart"] = multipart
        elif raw_body is not None:
            request["body_bytes"] = len(raw_body)
            request["body_sha256"] = sha256_hex(raw_body)

        filename = self._name(name, ".json")
        record = {
            "name": name,
            "request": request,
            "response": response,
        }
        (self.case_dir / filename).write_text(
            json.dumps(record, ensure_ascii=False, indent=2) + "\n"
        )
        self.index.append(
            {
                "kind": "http",
                "name": name,
                "file": f"cases/{filename}",
                "method": method,
                "path": path,
                "status": status,
            }
        )
        return status, response_headers, response_body, parsed

    def email(self, name: str, to: str, message: dict | None = None) -> dict:
        if message is None:
            if self.mailhog is None:
                raise RuntimeError("mailhog is not configured")
            message = self.mailhog.wait_for(to)
        parsed = MailHog.parsed(message)
        parsed["subject"] = normalize_email_text(parsed["subject"])
        parsed["body"] = normalize_email_text(parsed["body"])
        filename = self._name(name, ".email.json")
        (self.email_dir / filename).write_text(
            json.dumps(parsed, ensure_ascii=False, indent=2) + "\n"
        )
        self.index.append(
            {
                "kind": "email",
                "name": name,
                "file": f"emails/{filename}",
                "to": parsed["to"],
                "subject": parsed["subject"],
            }
        )
        return parsed

    def note(self, name: str, data) -> None:
        filename = self._name(name, ".note.json")
        (self.note_dir / filename).write_text(
            json.dumps(data, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
        )
        self.index.append(
            {"kind": "note", "name": name, "file": f"notes/{filename}"}
        )

    def redis_snapshot(self, name: str, email_address: str) -> None:
        if self.redis is None:
            raise RuntimeError("redis is not configured")
        self.note(name, redis_snapshot(self.redis, email_address))

    def finish(self, meta: dict | None = None) -> None:
        index = {
            "meta": meta or {},
            "count": len(self.index),
            "cases": self.index,
        }
        (self.out_dir / "index.json").write_text(
            json.dumps(index, ensure_ascii=False, indent=2) + "\n"
        )
