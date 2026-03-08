# Load Test Report — 2026-03-08 01:02:18 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 500 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 60.2s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 830.0 ops/sec |
| Error rate | 12.30% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 9893 | 406 | 82 | 506 | 1931 | 398 | 30085 |
| /v1/delete | 9894 | 83 | 36 | 97 | 370 | 318 | 30083 |
| /v1/discover | 2500 | 2500 | 0 | 1 | 33 | 1 | 171 |
| /v1/export | 2500 | 2500 | 38 | 78 | 30012 | 497 | 30191 |
| /v1/recall | 2500 | 44 | 51 | 126 | 30038 | 585 | 30195 |
| /v1/search | 2500 | 42 | 24 | 64 | 30017 | 531 | 30191 |
| /v1/store | 10320 | 402 | 36 | 245 | 30116 | 1273 | 30199 |
| /v1/update | 9893 | 175 | 138 | 720 | 2705 | 510 | 30274 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

