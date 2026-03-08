# Load Test Report — 2026-03-08 00:58:36 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 100 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 30.0s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 332.9 ops/sec |
| Error rate | 93.51% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 63 | 7 | 139 | 192 | 242 | 127 | 242 |
| /v1/delete | 63 | 0 | 79 | 132 | 156 | 82 | 156 |
| /v1/discover | 500 | 500 | 0 | 2 | 6 | 0 | 16 |
| /v1/export | 500 | 500 | 0 | 2 | 38 | 1 | 51 |
| /v1/recall | 500 | 300 | 0 | 120 | 148 | 34 | 228 |
| /v1/search | 500 | 302 | 0 | 66 | 139 | 136 | 30024 |
| /v1/store | 7811 | 7740 | 0 | 2 | 46 | 142 | 30024 |
| /v1/update | 63 | 2 | 133 | 203 | 235 | 132 | 235 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

