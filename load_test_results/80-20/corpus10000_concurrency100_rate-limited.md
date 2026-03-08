# Load Test Report — 2026-03-08 00:25:23 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 100 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 31.1s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 321.2 ops/sec |
| Error rate | 93.18% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 80 | 16 | 203 | 521 | 692 | 226 | 692 |
| /v1/delete | 80 | 0 | 91 | 340 | 445 | 127 | 445 |
| /v1/discover | 2000 | 2000 | 2 | 4 | 7 | 1 | 48 |
| /v1/export | 2000 | 2000 | 2 | 4 | 126 | 10 | 1327 |
| /v1/recall | 2000 | 1805 | 2 | 126 | 614 | 98 | 30159 |
| /v1/search | 2000 | 1813 | 2 | 108 | 427 | 210 | 30186 |
| /v1/store | 1760 | 1680 | 2 | 62 | 30029 | 352 | 30069 |
| /v1/update | 80 | 4 | 195 | 479 | 609 | 218 | 609 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

