# Load Test Report — 2026-03-08 00:45:19 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 50 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 2.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 2061.8 ops/sec |
| Error rate | 84.08% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 98 | 1 | 90 | 411 | 545 | 134 | 545 |
| /v1/delete | 100 | 0 | 85 | 320 | 341 | 108 | 346 |
| /v1/discover | 650 | 650 | 2 | 10 | 18 | 3 | 121 |
| /v1/export | 650 | 650 | 2 | 9 | 562 | 19 | 745 |
| /v1/recall | 600 | 400 | 4 | 164 | 279 | 42 | 381 |
| /v1/search | 600 | 400 | 4 | 127 | 203 | 31 | 296 |
| /v1/store | 2194 | 2094 | 2 | 14 | 137 | 6 | 350 |
| /v1/update | 108 | 9 | 120 | 418 | 524 | 154 | 527 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

