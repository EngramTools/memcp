# Load Test Report — 2026-03-08 00:36:51 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 50 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 5.7s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 873.3 ops/sec |
| Error rate | 26.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 588 | 0 | 68 | 230 | 364 | 88 | 466 |
| /v1/delete | 588 | 0 | 44 | 88 | 149 | 48 | 230 |
| /v1/discover | 650 | 650 | 0 | 4 | 32 | 1 | 115 |
| /v1/export | 650 | 650 | 13 | 29 | 59 | 15 | 103 |
| /v1/recall | 600 | 0 | 47 | 100 | 172 | 53 | 178 |
| /v1/search | 600 | 0 | 20 | 46 | 93 | 23 | 153 |
| /v1/store | 687 | 0 | 45 | 727 | 856 | 98 | 900 |
| /v1/update | 637 | 0 | 96 | 255 | 348 | 116 | 464 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

