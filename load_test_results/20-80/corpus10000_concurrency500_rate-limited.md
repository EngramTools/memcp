# Load Test Report — 2026-03-08 01:07:49 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 500 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 30.7s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 1631.1 ops/sec |
| Error rate | 99.67% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 13 | 10 | 744 | 30005 | 30005 | 9645 | 30005 |
| /v1/delete | 15 | 6 | 814 | 30005 | 30005 | 12376 | 30005 |
| /v1/discover | 2500 | 2500 | 3 | 19 | 90 | 6 | 181 |
| /v1/export | 2500 | 2500 | 3 | 18 | 166 | 222 | 30090 |
| /v1/recall | 2500 | 2439 | 3 | 30002 | 30075 | 1794 | 30447 |
| /v1/search | 2500 | 2444 | 3 | 30002 | 30074 | 1745 | 30350 |
| /v1/store | 39959 | 39930 | 3 | 18 | 102 | 70 | 30401 |
| /v1/update | 13 | 7 | 811 | 30005 | 30005 | 11906 | 30005 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

