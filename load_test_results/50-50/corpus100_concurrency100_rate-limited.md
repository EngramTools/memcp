# Load Test Report — 2026-03-08 00:37:55 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 100 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 30.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 328.9 ops/sec |
| Error rate | 93.21% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 71 | 7 | 152 | 683 | 30002 | 645 | 30002 |
| /v1/delete | 71 | 1 | 96 | 195 | 30002 | 521 | 30002 |
| /v1/discover | 1300 | 1300 | 2 | 12 | 39 | 4 | 86 |
| /v1/export | 1300 | 1300 | 2 | 12 | 77 | 28 | 30070 |
| /v1/recall | 1200 | 1003 | 4 | 130 | 219 | 122 | 30128 |
| /v1/search | 1200 | 1005 | 4 | 52 | 117 | 136 | 30114 |
| /v1/store | 4781 | 4704 | 3 | 13 | 256 | 189 | 30118 |
| /v1/update | 77 | 1 | 153 | 710 | 30002 | 630 | 30002 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

