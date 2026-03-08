# Load Test Report — 2026-03-08 00:57:30 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 10 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 1.0s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 1005.7 ops/sec |
| Error rate | 10.10% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 198 | 1 | 5 | 14 | 156 | 7 | 167 |
| /v1/delete | 198 | 0 | 6 | 11 | 50 | 7 | 86 |
| /v1/discover | 50 | 50 | 0 | 1 | 4 | 0 | 4 |
| /v1/export | 50 | 50 | 4 | 10 | 15 | 4 | 15 |
| /v1/recall | 50 | 0 | 9 | 24 | 99 | 12 | 99 |
| /v1/search | 50 | 0 | 5 | 10 | 17 | 5 | 17 |
| /v1/store | 207 | 0 | 6 | 114 | 188 | 15 | 238 |
| /v1/update | 197 | 0 | 9 | 16 | 24 | 10 | 27 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

