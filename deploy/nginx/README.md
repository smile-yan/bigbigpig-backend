# Nginx: Rust canary and cutover

`smileyan-backend-locations.conf` is a merge-in block: copy its `split_clients`,
`map`, and `location` blocks into the existing site `server { ... }`. Do not add
a second `listen`/`server_name`.

Ports:

| Process | Address |
|---|---|
| Go (rollback path, current prod) | `127.0.0.1:8080` |
| Rust staging | `127.0.0.1:8081` |
| Rust production after cutover | `127.0.0.1:8080` (Go stopped) |

## 1. Stage the canary

Deploy Rust to staging with `.github/workflows/staging.yml` (port 8081), then
run the smoke test:

```bash
ssh deploy@server 'bash /tmp/smoke-staging.sh http://127.0.0.1:8081'
# or locally:
./scripts/smoke.sh http://127.0.0.1:8081
```

Merge the location block with the 5% weight. Validate and reload:

```bash
sudo nginx -t
sudo systemctl reload nginx   # or: sudo nginx -s reload
```

## 2. Promote the canary

Change only the `split_clients` percentage, then `nginx -t` + reload:

| Stage | Weight | Hold time |
|---|---|---|
| 1 | `5% "127.0.0.1:8081";` | 30–60 min |
| 2 | `25% "127.0.0.1:8081";` | 1–2 h |
| 3 | `50% "127.0.0.1:8081";` | 2–4 h |
| 4 | cutover: stop Go, Rust binds 8080, proxy all traffic to Rust | — |

During stages 1–3 all non-GET methods go to Go, so writes never update the
Rust index. `/api/search` is also pinned to Go for that reason. After the
cutover:

1. Stop Go (its `stop.sh`, or `systemctl stop` if it is a unit).
2. Restart Rust with the prod config on **8080** (deploy to the production
   `DEPLOY_PATH` with `APP_PORT=8080`, or move the binary and restart).
3. Replace the canary block with direct proxying — delete `split_clients`,
   the `map`, and the `location = /api/search` override, and change the three
   `proxy_pass` lines to `http://127.0.0.1:8080`:

   ```nginx
   location /api/    { proxy_pass http://127.0.0.1:8080; ... }
   location /v1/     { proxy_pass http://127.0.0.1:8080; ... }
   location /uploads/ { proxy_pass http://127.0.0.1:8080; ... }
   ```

   Do not simply set the split to `100%` and leave the map default at 8080
   while Go is stopped: non-GET writes would target the stopped process.
4. `nginx -t` + reload; run `./scripts/smoke.sh http://127.0.0.1:8080`.

## 3. Rollback

Fastest, while the Go process is still running: revert the Nginx block and
reload (traffic returns to Go; no database or Redis change).

If Go was stopped and Rust took over 8080:

```bash
cd /opt/smileyan-backend
./rollback.sh          # restores smileyan-backend.prev + config.yaml.prev
```

Nginx does not need a change when Rust owns the port; `rollback.sh` health-checks
`/api/categories` and prints the restored binary hash.

## 4. Observation (7 days)

Use the site's access log. If it does not already log upstream timings, add:

```nginx
log_format smileyan '$remote_addr - $remote_user [$time_local] "$request" '
                    '$status $body_bytes_sent "$http_referer" "$http_user_agent" '
                    'rt=$request_time urt=$upstream_response_time';
access_log /var/log/nginx/smileyan-access.log smileyan;
```

Daily checks:

```bash
# 5xx rate (aim < 1%)
awk '{print $9}' /var/log/nginx/smileyan-access.log | sort | uniq -c
# slowest upstream responses
awk -F'urt=' '{print $2}' /var/log/nginx/smileyan-access.log | sort -nr | head
# backend errors / panics
grep -Ei 'panic|Failed to|ERROR' /opt/smileyan-backend/logs/*.log | tail -50
# memory and process
ps -o pid,rss,etime,args -C smileyan-backend
```

Also exercise login (verification email), post create/edit, comment, search and
TTS once per day; confirm `logs/YYYY-MM-DD.log` contains request entries with
`method/path/ip/status`.

## 5. Rollback triggers

- 5xx > 1% for 5 minutes.
- p95 latency > 2× the Go baseline for 10 minutes.
- Login, publishing, or commenting fails.
- Visible Markdown or search regression.

Keep the Go binary and its `.env` until the 7-day observation ends.
