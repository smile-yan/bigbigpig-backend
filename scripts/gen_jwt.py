#!/usr/bin/env python3
"""Generate deterministic JWTs for manual capture probes.

Examples:
  ./scripts/gen_jwt.py                       # admin / reader / dev tokens
  ./scripts/gen_jwt.py --user-id 2 --role reader --email reader@example.com
  ./scripts/gen_jwt.py --dev my-dev-token
"""
from __future__ import annotations

import argparse
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent / "lib"))

from capture_common import env, make_jwt  # noqa: E402

ISSUED_AT = 1_760_000_000
EXPIRES_AT = 2_000_000_000


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--secret", default=env("CAPTURE_JWT_SECRET", "capture-jwt-secret"))
    parser.add_argument("--user-id", type=int)
    parser.add_argument("--email")
    parser.add_argument("--role")
    parser.add_argument("--dev", help="build a dev token carrying this value")
    parser.add_argument("--alg", default="HS256", choices=["HS256", "HS512"])
    parser.add_argument("--exp", type=int, default=EXPIRES_AT)
    args = parser.parse_args()

    common = {"iat": ISSUED_AT, "nbf": ISSUED_AT, "exp": args.exp}
    if args.dev:
        print(make_jwt({"dev": args.dev, **common}, args.secret, args.alg))
        return
    if args.user_id is None:
        print("admin:", make_jwt({"user_id": 1, "email": "admin@example.com", "role": "admin", **common}, args.secret, args.alg))
        print("reader:", make_jwt({"user_id": 2, "email": "reader@example.com", "role": "reader", **common}, args.secret, args.alg))
        print("dev:", make_jwt({"dev": "capture-dev-token", **common}, args.secret, args.alg))
        return

    claims = {"user_id": args.user_id, **common}
    if args.email:
        claims["email"] = args.email
    if args.role:
        claims["role"] = args.role
    print(make_jwt(claims, args.secret, args.alg))


if __name__ == "__main__":
    main()
