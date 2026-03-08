# Load Test Report — 2026-03-08 00:18:10 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 10 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 0.6s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 1563.2 ops/sec |
| Error rate | 40.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 48 | 0 | 6 | 16 | 108 | 9 | 108 |
| /v1/delete | 48 | 0 | 7 | 11 | 108 | 11 | 108 |
| /v1/discover | 200 | 200 | 0 | 0 | 1 | 0 | 1 |
| /v1/export | 200 | 200 | 0 | 20 | 24 | 2 | 27 |
| /v1/recall | 200 | 0 | 8 | 18 | 22 | 8 | 27 |
| /v1/search | 200 | 0 | 5 | 9 | 12 | 5 | 13 |
| /v1/store | 57 | 0 | 7 | 107 | 111 | 12 | 111 |
| /v1/update | 47 | 0 | 10 | 113 | 114 | 19 | 114 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

