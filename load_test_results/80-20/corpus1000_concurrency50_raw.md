# Load Test Report — 2026-03-08 00:18:15 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 50 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 5.2s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 956.4 ops/sec |
| Error rate | 40.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 245 | 0 | 59 | 126 | 154 | 65 | 180 |
| /v1/delete | 245 | 0 | 68 | 99 | 117 | 70 | 133 |
| /v1/discover | 1000 | 1000 | 0 | 5 | 11 | 1 | 36 |
| /v1/export | 1000 | 1000 | 51 | 81 | 97 | 53 | 122 |
| /v1/recall | 1000 | 0 | 79 | 106 | 125 | 80 | 144 |
| /v1/search | 1000 | 0 | 36 | 59 | 76 | 37 | 97 |
| /v1/store | 265 | 0 | 66 | 92 | 110 | 68 | 120 |
| /v1/update | 245 | 0 | 110 | 173 | 240 | 115 | 262 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

