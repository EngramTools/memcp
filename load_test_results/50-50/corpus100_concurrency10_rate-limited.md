# Load Test Report — 2026-03-08 00:36:46 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 10 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 1.3s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 795.6 ops/sec |
| Error rate | 36.20% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 2 | 13 | 56 | 62 | 18 | 76 |
| /v1/delete | 100 | 0 | 10 | 34 | 37 | 15 | 48 |
| /v1/discover | 130 | 130 | 0 | 2 | 4 | 0 | 6 |
| /v1/export | 130 | 130 | 0 | 6 | 12 | 1 | 20 |
| /v1/recall | 120 | 0 | 24 | 43 | 60 | 23 | 60 |
| /v1/search | 120 | 0 | 10 | 22 | 24 | 10 | 25 |
| /v1/store | 193 | 93 | 4 | 31 | 44 | 7 | 60 |
| /v1/update | 107 | 7 | 18 | 62 | 65 | 23 | 80 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

