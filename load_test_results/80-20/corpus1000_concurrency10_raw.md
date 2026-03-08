# Load Test Report — 2026-03-08 00:18:10 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 10 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 1.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 709.4 ops/sec |
| Error rate | 40.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 48 | 0 | 8 | 29 | 30 | 11 | 30 |
| /v1/delete | 48 | 0 | 9 | 21 | 32 | 11 | 32 |
| /v1/discover | 200 | 200 | 0 | 1 | 1 | 0 | 2 |
| /v1/export | 200 | 200 | 23 | 32 | 38 | 23 | 55 |
| /v1/recall | 200 | 0 | 18 | 30 | 33 | 18 | 55 |
| /v1/search | 200 | 0 | 9 | 19 | 24 | 10 | 65 |
| /v1/store | 57 | 0 | 10 | 35 | 46 | 13 | 46 |
| /v1/update | 47 | 0 | 14 | 25 | 31 | 15 | 31 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

