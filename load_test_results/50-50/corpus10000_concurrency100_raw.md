# Load Test Report — 2026-03-08 00:46:59 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 100 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 100.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 99.6 ops/sec |
| Error rate | 26.95% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 1182 | 10 | 395 | 994 | 30110 | 898 | 30681 |
| /v1/delete | 1184 | 11 | 382 | 862 | 30148 | 950 | 60094 |
| /v1/discover | 1300 | 1300 | 3 | 119 | 257 | 22 | 651 |
| /v1/export | 1300 | 1300 | 752 | 1329 | 1797 | 952 | 30042 |
| /v1/recall | 1200 | 13 | 458 | 992 | 30002 | 871 | 60316 |
| /v1/search | 1200 | 8 | 263 | 726 | 1393 | 509 | 30124 |
| /v1/store | 1352 | 34 | 368 | 981 | 30051 | 1237 | 30282 |
| /v1/update | 1282 | 19 | 541 | 1050 | 30092 | 1062 | 30377 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

