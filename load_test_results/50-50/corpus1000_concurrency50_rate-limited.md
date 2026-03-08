# Load Test Report — 2026-03-08 00:39:43 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 50 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 1.6s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 3167.0 ops/sec |
| Error rate | 84.06% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 3 | 85 | 279 | 316 | 110 | 334 |
| /v1/delete | 100 | 0 | 53 | 182 | 211 | 72 | 260 |
| /v1/discover | 650 | 650 | 1 | 5 | 10 | 1 | 20 |
| /v1/export | 650 | 650 | 1 | 7 | 97 | 4 | 140 |
| /v1/recall | 600 | 400 | 3 | 118 | 155 | 27 | 176 |
| /v1/search | 600 | 400 | 3 | 68 | 85 | 14 | 122 |
| /v1/store | 2192 | 2092 | 2 | 12 | 77 | 4 | 131 |
| /v1/update | 108 | 8 | 109 | 300 | 316 | 125 | 322 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

