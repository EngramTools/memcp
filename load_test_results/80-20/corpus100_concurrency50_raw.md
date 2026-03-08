# Load Test Report — 2026-03-08 00:15:29 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 50 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 4.5s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 1104.4 ops/sec |
| Error rate | 40.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 245 | 0 | 72 | 302 | 434 | 106 | 464 |
| /v1/delete | 245 | 0 | 47 | 120 | 146 | 56 | 189 |
| /v1/discover | 1000 | 1000 | 0 | 2 | 13 | 0 | 30 |
| /v1/export | 1000 | 1000 | 12 | 36 | 95 | 19 | 725 |
| /v1/recall | 1000 | 0 | 45 | 111 | 698 | 61 | 785 |
| /v1/search | 1000 | 0 | 19 | 56 | 695 | 32 | 962 |
| /v1/store | 265 | 0 | 51 | 716 | 868 | 109 | 1001 |
| /v1/update | 245 | 0 | 135 | 352 | 520 | 160 | 701 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

