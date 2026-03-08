# Load Test Report — 2026-03-08 00:15:24 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 10 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 0.7s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 1492.4 ops/sec |
| Error rate | 40.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 48 | 0 | 9 | 30 | 45 | 12 | 45 |
| /v1/delete | 48 | 0 | 10 | 15 | 52 | 11 | 52 |
| /v1/discover | 200 | 200 | 0 | 0 | 1 | 0 | 1 |
| /v1/export | 200 | 200 | 0 | 4 | 7 | 0 | 9 |
| /v1/recall | 200 | 0 | 11 | 17 | 19 | 11 | 24 |
| /v1/search | 200 | 0 | 4 | 8 | 11 | 4 | 13 |
| /v1/store | 57 | 0 | 10 | 49 | 112 | 15 | 112 |
| /v1/update | 47 | 0 | 15 | 32 | 51 | 17 | 51 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

