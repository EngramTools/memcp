# Load Test Report — 2026-03-08 00:43:22 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 500 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 30.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 1645.0 ops/sec |
| Error rate | 99.75% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 10 | 9 | 473 | 30008 | 30008 | 3436 | 30008 |
| /v1/delete | 10 | 1 | 358 | 30045 | 30045 | 3298 | 30045 |
| /v1/discover | 6500 | 6500 | 6 | 37 | 88 | 11 | 216 |
| /v1/export | 6500 | 6500 | 5 | 35 | 125 | 29 | 30055 |
| /v1/recall | 6000 | 5958 | 6 | 54 | 30015 | 808 | 30227 |
| /v1/search | 6000 | 5958 | 6 | 55 | 30018 | 803 | 30295 |
| /v1/store | 24970 | 24946 | 5 | 39 | 99 | 119 | 30287 |
| /v1/update | 10 | 5 | 451 | 30008 | 30008 | 3414 | 30008 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

