# Load Test Report — 2026-03-08 00:17:35 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 500 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 60.2s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 830.2 ops/sec |
| Error rate | 41.36% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 2465 | 75 | 93 | 1280 | 3224 | 530 | 30021 |
| /v1/delete | 2465 | 23 | 44 | 155 | 30000 | 360 | 30101 |
| /v1/discover | 10000 | 10000 | 0 | 1 | 32 | 1 | 194 |
| /v1/export | 10000 | 10000 | 12 | 50 | 30003 | 550 | 30183 |
| /v1/recall | 10000 | 212 | 42 | 147 | 30025 | 700 | 60114 |
| /v1/search | 10000 | 208 | 17 | 79 | 30024 | 647 | 30189 |
| /v1/store | 2605 | 124 | 44 | 30002 | 30083 | 1734 | 30157 |
| /v1/update | 2465 | 40 | 185 | 2203 | 30000 | 824 | 30182 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

