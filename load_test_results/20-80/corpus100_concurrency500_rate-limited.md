# Load Test Report — 2026-03-08 01:00:07 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 500 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 30.7s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 1626.7 ops/sec |
| Error rate | 99.51% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 29 | 29 | 30050 | 30335 | 30404 | 29071 | 30404 |
| /v1/delete | 31 | 18 | 30032 | 30126 | 30158 | 22353 | 30158 |
| /v1/discover | 2500 | 2500 | 1 | 13 | 85 | 4 | 197 |
| /v1/export | 2500 | 2500 | 1 | 13 | 130 | 76 | 30080 |
| /v1/recall | 2500 | 2414 | 1 | 30001 | 30131 | 1529 | 30327 |
| /v1/search | 2500 | 2408 | 1 | 347 | 30042 | 1307 | 30219 |
| /v1/store | 39914 | 39869 | 0 | 13 | 90 | 55 | 30196 |
| /v1/update | 26 | 16 | 30032 | 30148 | 30165 | 20903 | 30165 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

