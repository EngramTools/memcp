# Load Test Report — 2026-03-08 01:07:18 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 500 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 150.8s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 331.6 ops/sec |
| Error rate | 14.26% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 9724 | 448 | 220 | 826 | 30074 | 976 | 60399 |
| /v1/delete | 9730 | 188 | 178 | 429 | 30062 | 861 | 60282 |
| /v1/discover | 2500 | 2500 | 1 | 16 | 91 | 5 | 369 |
| /v1/export | 2500 | 2500 | 373 | 715 | 30070 | 1368 | 30358 |
| /v1/recall | 2500 | 99 | 220 | 647 | 30216 | 1521 | 60685 |
| /v1/search | 2500 | 99 | 129 | 448 | 30138 | 1333 | 30416 |
| /v1/store | 10831 | 1023 | 182 | 30073 | 30281 | 3155 | 30560 |
| /v1/update | 9715 | 275 | 357 | 998 | 30117 | 1169 | 30969 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

