# Load Test Report — 2026-03-08 00:47:32 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 100 clients |
| R/W ratio | 50/50 |
| Mode | rate-limited |
| Duration | 32.9s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 304.4 ops/sec |
| Error rate | 93.26% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 76 | 11 | 161 | 882 | 30546 | 1427 | 30546 |
| /v1/delete | 76 | 0 | 96 | 626 | 695 | 168 | 695 |
| /v1/discover | 1300 | 1300 | 2 | 9 | 75 | 4 | 212 |
| /v1/export | 1300 | 1300 | 2 | 10 | 956 | 18 | 1226 |
| /v1/recall | 1200 | 1009 | 2 | 241 | 869 | 260 | 30268 |
| /v1/search | 1200 | 1010 | 2 | 188 | 541 | 278 | 30248 |
| /v1/store | 4769 | 4693 | 2 | 10 | 668 | 162 | 30265 |
| /v1/update | 79 | 3 | 143 | 803 | 30074 | 608 | 30074 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

