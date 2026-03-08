# Load Test Report — 2026-03-08 00:20:52 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 500 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 90.3s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 553.6 ops/sec |
| Error rate | 42.26% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 2444 | 217 | 148 | 616 | 30048 | 685 | 60126 |
| /v1/delete | 2444 | 34 | 146 | 381 | 30067 | 636 | 60133 |
| /v1/discover | 10000 | 10000 | 1 | 20 | 67 | 4 | 175 |
| /v1/export | 10000 | 10000 | 78 | 239 | 30090 | 935 | 30187 |
| /v1/recall | 10000 | 311 | 165 | 544 | 30114 | 1165 | 60187 |
| /v1/search | 10000 | 298 | 81 | 287 | 30096 | 990 | 30201 |
| /v1/store | 2669 | 204 | 151 | 30050 | 30146 | 2487 | 30205 |
| /v1/update | 2443 | 65 | 245 | 671 | 30093 | 786 | 30426 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

