# Load Test Report — 2026-03-08 00:57:35 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 50 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 5.3s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 950.4 ops/sec |
| Error rate | 10.02% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 988 | 1 | 21 | 267 | 446 | 66 | 762 |
| /v1/delete | 988 | 0 | 10 | 27 | 57 | 12 | 113 |
| /v1/discover | 250 | 250 | 0 | 0 | 1 | 0 | 5 |
| /v1/export | 250 | 250 | 6 | 14 | 28 | 6 | 31 |
| /v1/recall | 250 | 0 | 15 | 33 | 72 | 17 | 85 |
| /v1/search | 250 | 0 | 7 | 18 | 40 | 8 | 42 |
| /v1/store | 1037 | 0 | 10 | 114 | 654 | 41 | 726 |
| /v1/update | 987 | 0 | 79 | 409 | 610 | 131 | 775 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

