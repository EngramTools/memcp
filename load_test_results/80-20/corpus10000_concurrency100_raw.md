# Load Test Report — 2026-03-08 00:24:52 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 100 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 107.6s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 92.9 ops/sec |
| Error rate | 40.88% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 490 | 2 | 323 | 861 | 30001 | 684 | 30329 |
| /v1/delete | 490 | 5 | 500 | 941 | 30017 | 901 | 30469 |
| /v1/discover | 2000 | 2000 | 3 | 106 | 272 | 20 | 584 |
| /v1/export | 2000 | 2000 | 977 | 1870 | 3369 | 1238 | 30107 |
| /v1/recall | 2000 | 32 | 663 | 1271 | 30053 | 1220 | 30757 |
| /v1/search | 2000 | 17 | 352 | 817 | 1295 | 647 | 30186 |
| /v1/store | 530 | 25 | 498 | 30005 | 30062 | 2030 | 30355 |
| /v1/update | 490 | 7 | 695 | 1253 | 30107 | 1146 | 30377 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

