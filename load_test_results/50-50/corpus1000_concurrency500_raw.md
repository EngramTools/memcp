# Load Test Report — 2026-03-08 00:42:51 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 500 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 120.6s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 414.7 ops/sec |
| Error rate | 28.80% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 5928 | 169 | 228 | 1028 | 30076 | 1210 | 30855 |
| /v1/delete | 5928 | 155 | 126 | 422 | 30063 | 1010 | 60232 |
| /v1/discover | 6500 | 6500 | 1 | 21 | 119 | 5 | 250 |
| /v1/export | 6500 | 6500 | 89 | 319 | 30096 | 1078 | 30250 |
| /v1/recall | 6000 | 241 | 152 | 551 | 30135 | 1399 | 30416 |
| /v1/search | 6000 | 221 | 78 | 330 | 30125 | 1198 | 30328 |
| /v1/store | 6722 | 416 | 131 | 30043 | 30191 | 2059 | 30370 |
| /v1/update | 6422 | 198 | 322 | 1132 | 30075 | 1312 | 30791 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

