# Load Test Report — 2026-03-08 01:03:32 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 10 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 2.8s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 360.1 ops/sec |
| Error rate | 10.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 198 | 0 | 7 | 28 | 87 | 10 | 104 |
| /v1/delete | 198 | 0 | 10 | 34 | 102 | 14 | 128 |
| /v1/discover | 50 | 50 | 0 | 2 | 2 | 0 | 2 |
| /v1/export | 50 | 50 | 189 | 267 | 344 | 193 | 344 |
| /v1/recall | 50 | 0 | 41 | 146 | 154 | 49 | 154 |
| /v1/search | 50 | 0 | 28 | 128 | 138 | 39 | 138 |
| /v1/store | 207 | 0 | 9 | 37 | 77 | 13 | 114 |
| /v1/update | 197 | 0 | 12 | 49 | 128 | 17 | 131 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

