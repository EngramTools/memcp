# Load Test Report — 2026-03-08 00:57:30 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 10 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 0.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 2401.5 ops/sec |
| Error rate | 50.30% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 2 | 6 | 13 | 17 | 6 | 20 |
| /v1/delete | 100 | 0 | 6 | 10 | 14 | 6 | 15 |
| /v1/discover | 50 | 50 | 0 | 0 | 1 | 0 | 1 |
| /v1/export | 50 | 50 | 0 | 6 | 7 | 1 | 7 |
| /v1/recall | 50 | 0 | 7 | 10 | 18 | 7 | 18 |
| /v1/search | 50 | 0 | 3 | 6 | 7 | 3 | 7 |
| /v1/store | 501 | 401 | 0 | 7 | 10 | 1 | 15 |
| /v1/update | 99 | 0 | 10 | 16 | 20 | 10 | 20 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

