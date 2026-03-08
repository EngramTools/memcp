# Load Test Report — 2026-03-08 00:58:06 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 100 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 30.0s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 332.8 ops/sec |
| Error rate | 10.51% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 1978 | 0 | 35 | 251 | 416 | 71 | 755 |
| /v1/delete | 1978 | 0 | 9 | 23 | 60 | 11 | 117 |
| /v1/discover | 500 | 500 | 0 | 0 | 2 | 0 | 37 |
| /v1/export | 500 | 500 | 6 | 13 | 36 | 6 | 90 |
| /v1/recall | 500 | 0 | 14 | 28 | 93 | 17 | 278 |
| /v1/search | 500 | 4 | 6 | 15 | 89 | 248 | 30021 |
| /v1/store | 2067 | 47 | 9 | 64 | 30019 | 697 | 30034 |
| /v1/update | 1977 | 0 | 112 | 420 | 645 | 149 | 1034 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

