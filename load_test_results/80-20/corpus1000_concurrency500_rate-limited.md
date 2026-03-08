# Load Test Report — 2026-03-08 00:21:22 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 500 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 30.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 1646.4 ops/sec |
| Error rate | 99.81% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 1 | 1 | 431 | 431 | 431 | 431 | 431 |
| /v1/delete | 3 | 0 | 303 | 360 | 360 | 320 | 360 |
| /v1/discover | 10000 | 10000 | 4 | 25 | 81 | 8 | 282 |
| /v1/export | 10000 | 10000 | 4 | 25 | 109 | 8 | 316 |
| /v1/recall | 10000 | 9953 | 4 | 30 | 30019 | 469 | 30271 |
| /v1/search | 10000 | 9960 | 4 | 29 | 30022 | 489 | 30292 |
| /v1/store | 9994 | 9991 | 4 | 27 | 597 | 299 | 30160 |
| /v1/update | 2 | 0 | 313 | 360 | 360 | 336 | 360 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

