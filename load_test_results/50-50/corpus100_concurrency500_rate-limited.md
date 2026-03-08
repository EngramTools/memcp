# Load Test Report — 2026-03-08 00:39:26 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 500 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 30.6s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 1632.0 ops/sec |
| Error rate | 99.70% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 10 | 8 | 30002 | 30072 | 30072 | 24099 | 30072 |
| /v1/delete | 10 | 9 | 30001 | 30003 | 30003 | 27022 | 30003 |
| /v1/discover | 6500 | 6500 | 6 | 41 | 89 | 11 | 311 |
| /v1/export | 6500 | 6500 | 6 | 41 | 111 | 62 | 30069 |
| /v1/recall | 6000 | 5944 | 6 | 53 | 30069 | 754 | 30375 |
| /v1/search | 6000 | 5940 | 6 | 52 | 30062 | 712 | 30262 |
| /v1/store | 24972 | 24942 | 6 | 39 | 106 | 119 | 30370 |
| /v1/update | 8 | 8 | 30002 | 30049 | 30049 | 30008 | 30049 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

