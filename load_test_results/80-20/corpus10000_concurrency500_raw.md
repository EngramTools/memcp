# Load Test Report — 2026-03-08 00:32:59 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 500 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 455.6s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 109.7 ops/sec |
| Error rate | 49.62% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 2296 | 385 | 914 | 30325 | 31781 | 4599 | 61316 |
| /v1/delete | 2298 | 243 | 968 | 30238 | 31478 | 4598 | 61132 |
| /v1/discover | 10000 | 10000 | 12 | 240 | 525 | 51 | 1422 |
| /v1/export | 10000 | 10000 | 1228 | 30213 | 30650 | 4903 | 31476 |
| /v1/recall | 10000 | 1655 | 1248 | 30524 | 31904 | 6487 | 61983 |
| /v1/search | 10000 | 1458 | 668 | 30293 | 30877 | 4999 | 32225 |
| /v1/store | 3112 | 777 | 1173 | 30684 | 31054 | 8738 | 32171 |
| /v1/update | 2294 | 290 | 1256 | 30263 | 31117 | 4721 | 32751 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

