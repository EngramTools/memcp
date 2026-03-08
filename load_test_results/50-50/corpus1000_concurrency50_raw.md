# Load Test Report — 2026-03-08 00:39:41 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 50 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 7.3s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 688.6 ops/sec |
| Error rate | 26.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 588 | 0 | 74 | 210 | 250 | 94 | 411 |
| /v1/delete | 588 | 0 | 67 | 103 | 129 | 68 | 166 |
| /v1/discover | 650 | 650 | 1 | 10 | 28 | 2 | 69 |
| /v1/export | 650 | 650 | 59 | 107 | 158 | 63 | 236 |
| /v1/recall | 600 | 0 | 97 | 143 | 173 | 98 | 178 |
| /v1/search | 600 | 0 | 51 | 90 | 103 | 53 | 112 |
| /v1/store | 687 | 0 | 63 | 104 | 136 | 67 | 220 |
| /v1/update | 637 | 0 | 119 | 205 | 253 | 124 | 433 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

