# Load Test Report — 2026-03-08 00:23:02 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 50 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 46.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 107.7 ops/sec |
| Error rate | 40.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 245 | 0 | 239 | 562 | 724 | 282 | 918 |
| /v1/delete | 245 | 0 | 372 | 848 | 1023 | 434 | 1123 |
| /v1/discover | 1000 | 1000 | 3 | 115 | 270 | 19 | 590 |
| /v1/export | 1000 | 1000 | 897 | 1430 | 1724 | 944 | 2823 |
| /v1/recall | 1000 | 0 | 518 | 925 | 1176 | 559 | 1358 |
| /v1/search | 1000 | 0 | 288 | 699 | 849 | 333 | 1087 |
| /v1/store | 265 | 0 | 362 | 742 | 1059 | 396 | 1210 |
| /v1/update | 245 | 0 | 551 | 1006 | 1358 | 593 | 1550 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

