# Load Test Report — 2026-03-08 01:00:47 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 100 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 30.0s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 333.0 ops/sec |
| Error rate | 10.67% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 1978 | 0 | 23 | 75 | 122 | 29 | 285 |
| /v1/delete | 1979 | 0 | 14 | 34 | 46 | 16 | 117 |
| /v1/discover | 500 | 500 | 0 | 1 | 4 | 0 | 11 |
| /v1/export | 500 | 500 | 29 | 46 | 93 | 30 | 109 |
| /v1/recall | 500 | 0 | 33 | 52 | 71 | 34 | 107 |
| /v1/search | 500 | 3 | 18 | 32 | 41 | 197 | 30021 |
| /v1/store | 2065 | 64 | 14 | 48 | 30015 | 946 | 30023 |
| /v1/update | 1978 | 0 | 48 | 114 | 176 | 54 | 239 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

