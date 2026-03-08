# Load Test Report — 2026-03-08 01:00:16 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 50 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 4.3s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 1164.5 ops/sec |
| Error rate | 10.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 988 | 0 | 38 | 156 | 234 | 53 | 433 |
| /v1/delete | 988 | 0 | 17 | 40 | 54 | 20 | 76 |
| /v1/discover | 250 | 250 | 0 | 1 | 1 | 0 | 1 |
| /v1/export | 250 | 250 | 30 | 50 | 64 | 31 | 71 |
| /v1/recall | 250 | 0 | 34 | 51 | 76 | 35 | 80 |
| /v1/search | 250 | 0 | 15 | 31 | 49 | 16 | 61 |
| /v1/store | 1037 | 0 | 17 | 40 | 53 | 20 | 76 |
| /v1/update | 987 | 0 | 75 | 241 | 386 | 97 | 592 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

