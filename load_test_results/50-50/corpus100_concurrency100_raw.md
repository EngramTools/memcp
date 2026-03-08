# Load Test Report — 2026-03-08 00:37:24 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 100 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 31.5s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 317.8 ops/sec |
| Error rate | 26.30% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 1183 | 1 | 80 | 302 | 556 | 135 | 30057 |
| /v1/delete | 1183 | 1 | 48 | 107 | 190 | 80 | 30137 |
| /v1/discover | 1300 | 1300 | 1 | 12 | 28 | 2 | 101 |
| /v1/export | 1300 | 1300 | 19 | 53 | 164 | 117 | 30070 |
| /v1/recall | 1200 | 6 | 55 | 124 | 413 | 215 | 30057 |
| /v1/search | 1200 | 1 | 26 | 65 | 270 | 58 | 30072 |
| /v1/store | 1354 | 18 | 49 | 317 | 30057 | 554 | 30304 |
| /v1/update | 1280 | 3 | 103 | 382 | 648 | 212 | 30100 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

