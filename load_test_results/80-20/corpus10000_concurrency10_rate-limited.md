# Load Test Report — 2026-03-08 00:22:16 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 10 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 2.0s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 509.0 ops/sec |
| Error rate | 40.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 48 | 0 | 8 | 28 | 39 | 12 | 39 |
| /v1/delete | 48 | 0 | 14 | 28 | 51 | 16 | 51 |
| /v1/discover | 200 | 200 | 0 | 1 | 2 | 0 | 9 |
| /v1/export | 200 | 200 | 0 | 242 | 281 | 24 | 352 |
| /v1/recall | 200 | 0 | 19 | 70 | 97 | 24 | 115 |
| /v1/search | 200 | 0 | 25 | 60 | 72 | 30 | 159 |
| /v1/store | 57 | 0 | 10 | 23 | 49 | 12 | 49 |
| /v1/update | 47 | 0 | 15 | 29 | 44 | 16 | 44 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

