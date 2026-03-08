# Load Test Report — 2026-03-08 01:04:17 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 100 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 30.0s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 332.9 ops/sec |
| Error rate | 10.71% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 1979 | 0 | 44 | 130 | 188 | 54 | 325 |
| /v1/delete | 1979 | 0 | 36 | 129 | 172 | 50 | 300 |
| /v1/discover | 500 | 500 | 1 | 7 | 15 | 1 | 32 |
| /v1/export | 500 | 500 | 232 | 354 | 420 | 243 | 580 |
| /v1/recall | 500 | 1 | 95 | 182 | 220 | 163 | 30018 |
| /v1/search | 500 | 4 | 52 | 105 | 219 | 295 | 30015 |
| /v1/store | 2064 | 66 | 37 | 156 | 30018 | 1009 | 30020 |
| /v1/update | 1978 | 0 | 98 | 207 | 270 | 104 | 430 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

