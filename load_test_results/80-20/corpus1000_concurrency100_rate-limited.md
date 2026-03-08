# Load Test Report — 2026-03-08 00:19:22 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 100 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 30.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 329.4 ops/sec |
| Error rate | 93.06% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 80 | 10 | 97 | 451 | 30107 | 518 | 30107 |
| /v1/delete | 80 | 0 | 37 | 84 | 139 | 42 | 139 |
| /v1/discover | 2000 | 2000 | 1 | 3 | 8 | 1 | 10 |
| /v1/export | 2000 | 2000 | 1 | 3 | 9 | 2 | 172 |
| /v1/recall | 2000 | 1804 | 1 | 54 | 195 | 67 | 30025 |
| /v1/search | 2000 | 1813 | 1 | 27 | 101 | 199 | 30036 |
| /v1/store | 1760 | 1676 | 1 | 19 | 30002 | 345 | 30040 |
| /v1/update | 80 | 3 | 110 | 395 | 30057 | 512 | 30057 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

