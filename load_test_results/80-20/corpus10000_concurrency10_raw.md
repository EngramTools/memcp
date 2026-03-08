# Load Test Report — 2026-03-08 00:22:14 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 10 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 9.2s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 108.8 ops/sec |
| Error rate | 40.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 48 | 0 | 15 | 54 | 76 | 21 | 76 |
| /v1/delete | 48 | 0 | 16 | 67 | 69 | 24 | 69 |
| /v1/discover | 200 | 200 | 1 | 5 | 13 | 1 | 20 |
| /v1/export | 200 | 200 | 253 | 473 | 526 | 277 | 561 |
| /v1/recall | 200 | 0 | 68 | 179 | 392 | 81 | 411 |
| /v1/search | 200 | 0 | 45 | 99 | 339 | 56 | 359 |
| /v1/store | 57 | 0 | 26 | 63 | 163 | 33 | 163 |
| /v1/update | 47 | 0 | 24 | 79 | 176 | 35 | 176 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

