# Load Test Report — 2026-03-08 00:40:51 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 100 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 30.1s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 332.6 ops/sec |
| Error rate | 93.77% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 53 | 9 | 86 | 366 | 476 | 127 | 476 |
| /v1/delete | 54 | 0 | 38 | 131 | 226 | 55 | 226 |
| /v1/discover | 1300 | 1300 | 0 | 7 | 11 | 1 | 21 |
| /v1/export | 1300 | 1300 | 0 | 8 | 77 | 3 | 179 |
| /v1/recall | 1200 | 1001 | 1 | 76 | 259 | 39 | 30059 |
| /v1/search | 1200 | 1006 | 1 | 40 | 115 | 157 | 30050 |
| /v1/store | 4836 | 4757 | 0 | 8 | 180 | 287 | 30059 |
| /v1/update | 57 | 4 | 88 | 350 | 391 | 117 | 391 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

