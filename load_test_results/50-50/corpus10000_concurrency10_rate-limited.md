# Load Test Report — 2026-03-08 00:44:41 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 10 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 3.1s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 320.0 ops/sec |
| Error rate | 36.20% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 2 | 23 | 78 | 98 | 29 | 154 |
| /v1/delete | 100 | 0 | 23 | 68 | 108 | 28 | 127 |
| /v1/discover | 130 | 130 | 0 | 7 | 14 | 1 | 66 |
| /v1/export | 130 | 130 | 0 | 396 | 433 | 53 | 434 |
| /v1/recall | 120 | 0 | 34 | 96 | 143 | 42 | 147 |
| /v1/search | 120 | 0 | 32 | 68 | 89 | 36 | 93 |
| /v1/store | 193 | 93 | 6 | 56 | 136 | 14 | 156 |
| /v1/update | 107 | 7 | 37 | 105 | 160 | 42 | 279 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

