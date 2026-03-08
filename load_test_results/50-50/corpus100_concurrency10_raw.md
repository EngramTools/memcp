# Load Test Report — 2026-03-08 00:36:44 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 10 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 2.5s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 408.1 ops/sec |
| Error rate | 26.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 118 | 0 | 23 | 56 | 74 | 28 | 147 |
| /v1/delete | 118 | 0 | 23 | 44 | 48 | 24 | 57 |
| /v1/discover | 130 | 130 | 0 | 1 | 2 | 0 | 3 |
| /v1/export | 130 | 130 | 9 | 16 | 25 | 9 | 30 |
| /v1/recall | 120 | 0 | 29 | 46 | 54 | 29 | 100 |
| /v1/search | 120 | 0 | 13 | 24 | 30 | 13 | 36 |
| /v1/store | 137 | 0 | 22 | 246 | 334 | 41 | 334 |
| /v1/update | 127 | 0 | 37 | 77 | 124 | 41 | 143 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

