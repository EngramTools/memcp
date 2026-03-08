# Load Test Report — 2026-03-08 00:18:05 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 500 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 30.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 1646.0 ops/sec |
| Error rate | 99.72% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 9 | 6 | 30002 | 30080 | 30080 | 16769 | 30080 |
| /v1/delete | 9 | 6 | 30007 | 30058 | 30058 | 20069 | 30058 |
| /v1/discover | 10000 | 10000 | 4 | 19 | 76 | 7 | 202 |
| /v1/export | 10000 | 10000 | 4 | 20 | 79 | 7 | 199 |
| /v1/recall | 10000 | 9945 | 4 | 20 | 30058 | 485 | 30236 |
| /v1/search | 10000 | 9944 | 4 | 21 | 30042 | 439 | 30235 |
| /v1/store | 9973 | 9953 | 4 | 20 | 175 | 281 | 30241 |
| /v1/update | 9 | 6 | 30008 | 30103 | 30103 | 16763 | 30103 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

