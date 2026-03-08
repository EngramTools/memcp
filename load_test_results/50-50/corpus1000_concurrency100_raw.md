# Load Test Report — 2026-03-08 00:40:20 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 100 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 37.8s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 264.8 ops/sec |
| Error rate | 26.37% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 1183 | 5 | 100 | 300 | 541 | 273 | 30479 |
| /v1/delete | 1183 | 1 | 71 | 131 | 205 | 203 | 30244 |
| /v1/discover | 1300 | 1300 | 1 | 9 | 33 | 2 | 146 |
| /v1/export | 1300 | 1300 | 60 | 112 | 188 | 134 | 30006 |
| /v1/recall | 1200 | 8 | 104 | 167 | 248 | 333 | 30210 |
| /v1/search | 1200 | 3 | 54 | 100 | 184 | 132 | 30061 |
| /v1/store | 1354 | 17 | 72 | 157 | 30038 | 477 | 30083 |
| /v1/update | 1280 | 3 | 136 | 303 | 458 | 225 | 30069 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

