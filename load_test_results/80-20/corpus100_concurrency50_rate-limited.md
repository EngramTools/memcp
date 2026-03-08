# Load Test Report — 2026-03-08 00:15:29 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 50 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 0.9s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 5863.9 ops/sec |
| Error rate | 84.14% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 4 | 49 | 302 | 425 | 89 | 549 |
| /v1/delete | 100 | 0 | 23 | 58 | 91 | 27 | 98 |
| /v1/discover | 1000 | 1000 | 0 | 2 | 3 | 0 | 4 |
| /v1/export | 1000 | 1000 | 0 | 2 | 15 | 0 | 50 |
| /v1/recall | 1000 | 800 | 1 | 35 | 49 | 6 | 79 |
| /v1/search | 1000 | 800 | 1 | 15 | 25 | 3 | 46 |
| /v1/store | 700 | 600 | 1 | 31 | 56 | 5 | 83 |
| /v1/update | 100 | 3 | 85 | 289 | 429 | 113 | 431 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

