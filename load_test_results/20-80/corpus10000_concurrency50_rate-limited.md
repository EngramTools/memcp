# Load Test Report — 2026-03-08 01:03:47 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 50 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 1.8s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 2734.8 ops/sec |
| Error rate | 84.24% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 10 | 104 | 286 | 403 | 130 | 435 |
| /v1/delete | 100 | 0 | 76 | 158 | 174 | 82 | 202 |
| /v1/discover | 250 | 250 | 0 | 2 | 4 | 0 | 5 |
| /v1/export | 250 | 250 | 0 | 263 | 335 | 22 | 345 |
| /v1/recall | 250 | 50 | 65 | 139 | 197 | 64 | 213 |
| /v1/search | 250 | 50 | 71 | 201 | 277 | 84 | 446 |
| /v1/store | 3701 | 3601 | 0 | 3 | 55 | 1 | 115 |
| /v1/update | 99 | 1 | 134 | 272 | 357 | 147 | 357 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

