# Load Test Report — 2026-03-08 00:23:05 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 50 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 2.1s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 2369.2 ops/sec |
| Error rate | 84.24% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 9 | 55 | 219 | 623 | 93 | 645 |
| /v1/delete | 100 | 0 | 46 | 156 | 366 | 66 | 417 |
| /v1/discover | 1000 | 1000 | 1 | 2 | 6 | 1 | 73 |
| /v1/export | 1000 | 1000 | 1 | 3 | 805 | 16 | 1080 |
| /v1/recall | 1000 | 800 | 1 | 98 | 414 | 25 | 456 |
| /v1/search | 1000 | 800 | 1 | 109 | 320 | 20 | 431 |
| /v1/store | 700 | 600 | 1 | 82 | 220 | 13 | 485 |
| /v1/update | 100 | 3 | 87 | 229 | 603 | 114 | 613 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

