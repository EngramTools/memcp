# Load Test Report — 2026-03-08 00:59:37 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 500 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 60.2s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 830.5 ops/sec |
| Error rate | 11.53% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 9891 | 86 | 47 | 693 | 2393 | 424 | 30093 |
| /v1/delete | 9893 | 89 | 12 | 38 | 304 | 288 | 30101 |
| /v1/discover | 2500 | 2500 | 0 | 0 | 6 | 1 | 163 |
| /v1/export | 2500 | 2500 | 8 | 27 | 30001 | 407 | 30155 |
| /v1/recall | 2500 | 41 | 17 | 57 | 30003 | 526 | 60133 |
| /v1/search | 2500 | 42 | 8 | 33 | 30004 | 515 | 30149 |
| /v1/store | 10325 | 416 | 11 | 202 | 30106 | 1273 | 30163 |
| /v1/update | 9891 | 91 | 215 | 1195 | 2749 | 615 | 30123 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

