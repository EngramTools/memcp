# Load Test Report — 2026-03-08 01:00:17 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 50 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 0.8s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 6305.1 ops/sec |
| Error rate | 84.06% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 100 | 1 | 55 | 126 | 142 | 57 | 179 |
| /v1/delete | 100 | 0 | 36 | 94 | 158 | 38 | 186 |
| /v1/discover | 250 | 250 | 0 | 1 | 1 | 0 | 2 |
| /v1/export | 250 | 250 | 0 | 31 | 45 | 2 | 48 |
| /v1/recall | 250 | 50 | 48 | 81 | 85 | 45 | 90 |
| /v1/search | 250 | 50 | 22 | 35 | 39 | 19 | 41 |
| /v1/store | 3701 | 3601 | 0 | 1 | 22 | 0 | 51 |
| /v1/update | 99 | 1 | 63 | 118 | 163 | 66 | 163 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

