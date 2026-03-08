# Load Test Report — 2026-03-08 00:39:34 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 10 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 1.9s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 515.7 ops/sec |
| Error rate | 36.10% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 1 | 15 | 59 | 91 | 22 | 165 |
| /v1/delete | 100 | 0 | 18 | 54 | 91 | 22 | 98 |
| /v1/discover | 130 | 130 | 0 | 6 | 16 | 2 | 135 |
| /v1/export | 130 | 130 | 0 | 84 | 180 | 13 | 182 |
| /v1/recall | 120 | 0 | 20 | 88 | 108 | 27 | 121 |
| /v1/search | 120 | 0 | 11 | 44 | 84 | 15 | 85 |
| /v1/store | 193 | 93 | 6 | 39 | 83 | 11 | 140 |
| /v1/update | 107 | 7 | 26 | 157 | 208 | 39 | 213 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

