# Load Test Report — 2026-03-08 01:00:12 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 10 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 1.3s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 740.8 ops/sec |
| Error rate | 10.10% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 198 | 1 | 5 | 23 | 52 | 8 | 64 |
| /v1/delete | 198 | 0 | 7 | 28 | 32 | 10 | 42 |
| /v1/discover | 50 | 50 | 0 | 1 | 5 | 0 | 5 |
| /v1/export | 50 | 50 | 24 | 95 | 138 | 30 | 138 |
| /v1/recall | 50 | 0 | 22 | 139 | 146 | 32 | 146 |
| /v1/search | 50 | 0 | 14 | 45 | 108 | 16 | 108 |
| /v1/store | 207 | 0 | 7 | 33 | 47 | 10 | 50 |
| /v1/update | 197 | 0 | 10 | 41 | 54 | 14 | 118 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

