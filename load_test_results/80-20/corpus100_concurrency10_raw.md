# Load Test Report — 2026-03-08 00:15:23 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 10 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 1.1s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 881.9 ops/sec |
| Error rate | 40.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 48 | 0 | 7 | 27 | 32 | 10 | 32 |
| /v1/delete | 48 | 0 | 9 | 20 | 23 | 10 | 23 |
| /v1/discover | 200 | 200 | 0 | 1 | 3 | 0 | 7 |
| /v1/export | 200 | 200 | 5 | 10 | 14 | 6 | 200 |
| /v1/recall | 200 | 0 | 10 | 28 | 215 | 18 | 328 |
| /v1/search | 200 | 0 | 5 | 14 | 198 | 8 | 223 |
| /v1/store | 57 | 0 | 9 | 176 | 184 | 34 | 184 |
| /v1/update | 47 | 0 | 12 | 28 | 101 | 15 | 101 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

