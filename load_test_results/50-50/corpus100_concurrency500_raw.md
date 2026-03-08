# Load Test Report — 2026-03-08 00:38:55 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 500 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 60.3s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 829.2 ops/sec |
| Error rate | 27.40% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 5928 | 52 | 136 | 626 | 2270 | 492 | 30252 |
| /v1/delete | 5928 | 50 | 50 | 157 | 523 | 352 | 30102 |
| /v1/discover | 6500 | 6500 | 0 | 3 | 37 | 2 | 246 |
| /v1/export | 6500 | 6500 | 24 | 79 | 30004 | 520 | 30244 |
| /v1/recall | 6000 | 100 | 58 | 191 | 30005 | 579 | 30243 |
| /v1/search | 6000 | 110 | 27 | 98 | 30006 | 586 | 30243 |
| /v1/store | 6722 | 321 | 51 | 712 | 30144 | 1532 | 30250 |
| /v1/update | 6422 | 67 | 196 | 729 | 30001 | 569 | 30521 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

