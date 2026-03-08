# Load Test Report — 2026-03-08 00:55:10 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 500 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 30.9s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 1617.0 ops/sec |
| Error rate | 99.75% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 6 | 4 | 690 | 30015 | 30015 | 5591 | 30015 |
| /v1/delete | 6 | 1 | 792 | 30012 | 30012 | 5595 | 30012 |
| /v1/discover | 6500 | 6500 | 3 | 27 | 99 | 8 | 314 |
| /v1/export | 6500 | 6500 | 3 | 27 | 141 | 29 | 30022 |
| /v1/recall | 6000 | 5960 | 3 | 34 | 30089 | 818 | 30789 |
| /v1/search | 6000 | 5949 | 3 | 32 | 30068 | 758 | 30642 |
| /v1/store | 24982 | 24962 | 3 | 28 | 178 | 121 | 30875 |
| /v1/update | 6 | 1 | 614 | 828 | 828 | 674 | 828 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

