# Load Test Report — 2026-03-08 00:39:32 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 10 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 2.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 411.4 ops/sec |
| Error rate | 26.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 118 | 0 | 22 | 63 | 74 | 27 | 78 |
| /v1/delete | 118 | 0 | 20 | 43 | 49 | 20 | 52 |
| /v1/discover | 130 | 130 | 0 | 2 | 7 | 0 | 7 |
| /v1/export | 130 | 130 | 29 | 48 | 53 | 30 | 54 |
| /v1/recall | 120 | 0 | 30 | 53 | 59 | 33 | 61 |
| /v1/search | 120 | 0 | 17 | 29 | 38 | 17 | 39 |
| /v1/store | 137 | 0 | 20 | 39 | 50 | 21 | 54 |
| /v1/update | 127 | 0 | 32 | 58 | 71 | 34 | 78 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

