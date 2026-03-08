# Load Test Report — 2026-03-08 01:02:49 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 500 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 30.7s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 1626.8 ops/sec |
| Error rate | 99.57% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 30 | 22 | 30019 | 30196 | 30326 | 24178 | 30326 |
| /v1/delete | 35 | 25 | 30031 | 30218 | 30226 | 27547 | 30226 |
| /v1/discover | 2500 | 2500 | 2 | 16 | 62 | 4 | 212 |
| /v1/export | 2500 | 2500 | 2 | 16 | 110 | 136 | 30043 |
| /v1/recall | 2500 | 2436 | 2 | 30003 | 30120 | 1730 | 30279 |
| /v1/search | 2500 | 2413 | 2 | 323 | 30031 | 1367 | 30260 |
| /v1/store | 39908 | 39868 | 1 | 15 | 81 | 53 | 30261 |
| /v1/update | 27 | 20 | 30023 | 30171 | 30197 | 24620 | 30197 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

