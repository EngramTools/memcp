# Load Test Report — 2026-03-08 01:03:33 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 10 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 1.3s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 760.0 ops/sec |
| Error rate | 50.20% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 1 | 8 | 20 | 32 | 10 | 132 |
| /v1/delete | 100 | 0 | 10 | 27 | 42 | 13 | 125 |
| /v1/discover | 50 | 50 | 0 | 2 | 7 | 0 | 7 |
| /v1/export | 50 | 50 | 0 | 243 | 274 | 81 | 274 |
| /v1/recall | 50 | 0 | 17 | 89 | 145 | 30 | 145 |
| /v1/search | 50 | 0 | 20 | 68 | 125 | 28 | 125 |
| /v1/store | 501 | 401 | 0 | 13 | 34 | 2 | 130 |
| /v1/update | 99 | 0 | 12 | 43 | 130 | 16 | 130 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

