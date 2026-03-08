# Load Test Report — 2026-03-08 00:57:36 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 50 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 1.0s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 4995.7 ops/sec |
| Error rate | 84.12% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 5 | 66 | 338 | 489 | 104 | 519 |
| /v1/delete | 100 | 0 | 23 | 136 | 146 | 36 | 314 |
| /v1/discover | 250 | 250 | 0 | 1 | 1 | 0 | 4 |
| /v1/export | 250 | 250 | 0 | 8 | 23 | 1 | 29 |
| /v1/recall | 250 | 50 | 42 | 91 | 100 | 43 | 105 |
| /v1/search | 250 | 50 | 17 | 37 | 55 | 17 | 63 |
| /v1/store | 3701 | 3601 | 0 | 1 | 32 | 1 | 206 |
| /v1/update | 99 | 0 | 61 | 353 | 447 | 105 | 447 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

