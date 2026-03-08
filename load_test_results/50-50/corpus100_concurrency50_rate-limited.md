# Load Test Report — 2026-03-08 00:36:53 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 50 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 1.3s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 3724.0 ops/sec |
| Error rate | 84.06% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 3 | 85 | 307 | 465 | 114 | 590 |
| /v1/delete | 100 | 0 | 43 | 76 | 86 | 45 | 117 |
| /v1/discover | 650 | 650 | 2 | 8 | 11 | 2 | 57 |
| /v1/export | 650 | 650 | 2 | 9 | 14 | 3 | 20 |
| /v1/recall | 600 | 400 | 4 | 73 | 101 | 20 | 121 |
| /v1/search | 600 | 400 | 4 | 32 | 46 | 10 | 63 |
| /v1/store | 2192 | 2092 | 3 | 14 | 41 | 4 | 108 |
| /v1/update | 108 | 8 | 100 | 324 | 358 | 128 | 372 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

