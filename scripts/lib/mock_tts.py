#!/usr/bin/env python3
"""Deterministic OpenAI-compatible TTS upstream used by contract capture.

Triggers (based on the `input` field):

  TRIGGER_ERROR_400  -> 400 {"error": "mock bad request"}
  TRIGGER_ERROR_500  -> 500 {"error": "mock internal error"}
  TRIGGER_DROP       -> close the connection without a response
  anything else      -> binary audio, or SSE when stream_format == "sse"

GET /__last  returns the last decoded request (Authorization is reduced to a
boolean so the API key never lands in a fixture).
"""
from __future__ import annotations

import argparse
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BINARY_BODY = b"ID3\x04\x00\x00\x00\x00\x00\x00CAPTURE-AUDIO\x00\x01\x02\xff"
SSE_EVENTS = [
    {"type": "speech.audio.delta", "audio": "QUJD"},
    {"type": "speech.audio.delta", "audio": "REVG"},
    {"type": "speech.audio.done", "usage": {"input_tokens": 3, "output_tokens": 2, "total_tokens": 5}},
]

_last_lock = threading.Lock()
_last: dict = {}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # silence
        pass

    def _send_json(self, status: int, obj: dict) -> None:
        data = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/__last":
            with _last_lock:
                self._send_json(200, dict(_last))
        elif self.path == "/__reset":
            with _last_lock:
                _last.clear()
            self._send_json(200, {"reset": True})
        elif self.path == "/healthz":
            self._send_json(200, {"status": "ok"})
        else:
            self._send_json(404, {"error": "not found"})

    def do_POST(self) -> None:  # noqa: N802
        if self.path != "/v1/audio/speech":
            self._send_json(404, {"error": "not found"})
            return

        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        try:
            body = json.loads(raw)
        except json.JSONDecodeError:
            body = {"_raw": raw.decode("utf-8", "replace")}

        with _last_lock:
            _last.clear()
            _last.update(
                {
                    "headers": {
                        k.lower(): v
                        for k, v in self.headers.items()
                        if k.lower() not in {"authorization", "content-length", "user-agent", "host"}
                    },
                    "authorization_present": bool(self.headers.get("Authorization")),
                    "authorization_scheme": (self.headers.get("Authorization", "").split(" ")[0] or None),
                    "body": body,
                }
            )

        input_text = body.get("input", "")
        if input_text == "TRIGGER_ERROR_400":
            self._send_json(400, {"error": "mock bad request"})
            return
        if input_text == "TRIGGER_ERROR_500":
            self._send_json(500, {"error": "mock internal error"})
            return
        if input_text == "TRIGGER_DROP":
            self.close_connection = True
            try:
                self.connection.close()
            except OSError:
                pass
            return

        if body.get("stream_format") == "sse":
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Connection", "close")
            self.end_headers()
            self.close_connection = True
            try:
                for event in SSE_EVENTS:
                    payload = json.dumps(event, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
                    self.wfile.write(b"data: " + payload + b"\n\n")
                    self.wfile.flush()
                    time.sleep(0.05)
            except (BrokenPipeError, ConnectionResetError):
                pass
            return

        self.send_response(200)
        self.send_header("Content-Type", "audio/mpeg")
        self.send_header("Content-Length", str(len(BINARY_BODY)))
        self.end_headers()
        self.wfile.write(BINARY_BODY)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=19099)
    args = parser.parse_args()

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.daemon_threads = True
    print(f"mock TTS listening on http://{args.host}:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
