# Load Test Report — 2026-03-08 00:16:35 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 100 clients |
| R/W ratio | 80/20 |
| Mode | rate-limited |
| Duration | 30.1s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 331.8 ops/sec |
| Error rate | 92.97% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 83 | 6 | 128 | 375 | 555 | 148 | 555 |
| /v1/delete | 83 | 2 | 41 | 128 | 30028 | 766 | 30028 |
| /v1/discover | 2000 | 2000 | 1 | 3 | 14 | 1 | 74 |
| /v1/export | 2000 | 2000 | 1 | 3 | 17 | 2 | 93 |
| /v1/recall | 2000 | 1813 | 2 | 48 | 107 | 216 | 30101 |
| /v1/search | 2000 | 1809 | 2 | 19 | 89 | 138 | 30093 |
| /v1/store | 1751 | 1663 | 2 | 26 | 115 | 295 | 30082 |
| /v1/update | 83 | 4 | 132 | 507 | 609 | 171 | 609 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

