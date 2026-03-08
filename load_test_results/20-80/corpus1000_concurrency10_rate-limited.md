# Load Test Report — 2026-03-08 01:00:12 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 10 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 0.5s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 1931.8 ops/sec |
| Error rate | 50.10% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 0 | 5 | 18 | 26 | 6 | 26 |
| /v1/delete | 100 | 0 | 6 | 10 | 17 | 6 | 20 |
| /v1/discover | 50 | 50 | 0 | 0 | 0 | 0 | 0 |
| /v1/export | 50 | 50 | 0 | 33 | 37 | 9 | 37 |
| /v1/recall | 50 | 0 | 8 | 25 | 32 | 12 | 32 |
| /v1/search | 50 | 0 | 5 | 19 | 27 | 7 | 27 |
| /v1/store | 501 | 401 | 0 | 7 | 9 | 1 | 22 |
| /v1/update | 99 | 0 | 10 | 24 | 30 | 11 | 30 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

