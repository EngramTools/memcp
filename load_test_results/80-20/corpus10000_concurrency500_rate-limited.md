# Load Test Report — 2026-03-08 00:33:30 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 500 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 30.8s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 1624.4 ops/sec |
| Error rate | 99.79% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 6 | 6 | 289 | 325 | 325 | 286 | 325 |
| /v1/delete | 8 | 0 | 215 | 423 | 423 | 246 | 423 |
| /v1/discover | 10000 | 10000 | 4 | 23 | 108 | 9 | 682 |
| /v1/export | 10000 | 10000 | 4 | 22 | 205 | 12 | 2046 |
| /v1/recall | 10000 | 9959 | 4 | 24 | 30059 | 490 | 30538 |
| /v1/search | 10000 | 9959 | 4 | 24 | 30057 | 489 | 30679 |
| /v1/store | 9980 | 9968 | 4 | 21 | 777 | 286 | 30695 |
| /v1/update | 6 | 4 | 256 | 348 | 348 | 262 | 348 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

