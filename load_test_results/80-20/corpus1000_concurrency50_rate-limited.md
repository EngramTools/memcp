# Load Test Report — 2026-03-08 00:18:16 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 50 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 0.8s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 5951.5 ops/sec |
| Error rate | 84.20% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 4 | 55 | 210 | 330 | 77 | 405 |
| /v1/delete | 100 | 0 | 27 | 94 | 116 | 33 | 146 |
| /v1/discover | 1000 | 1000 | 0 | 1 | 2 | 0 | 6 |
| /v1/export | 1000 | 1000 | 0 | 1 | 48 | 1 | 63 |
| /v1/recall | 1000 | 800 | 0 | 39 | 84 | 7 | 113 |
| /v1/search | 1000 | 800 | 0 | 23 | 39 | 3 | 67 |
| /v1/store | 700 | 600 | 0 | 32 | 62 | 4 | 73 |
| /v1/update | 100 | 6 | 95 | 281 | 358 | 113 | 382 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

