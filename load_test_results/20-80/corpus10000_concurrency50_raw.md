# Load Test Report — 2026-03-08 01:03:45 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 50 clients |
| R/W ratio | 20/80 |
| Mode | raw |
| Duration | 12.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 402.9 ops/sec |
| Error rate | 10.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 988 | 0 | 91 | 282 | 459 | 115 | 706 |
| /v1/delete | 988 | 0 | 74 | 155 | 253 | 82 | 336 |
| /v1/discover | 250 | 250 | 1 | 6 | 9 | 1 | 25 |
| /v1/export | 250 | 250 | 288 | 424 | 557 | 298 | 574 |
| /v1/recall | 250 | 0 | 126 | 241 | 298 | 134 | 318 |
| /v1/search | 250 | 0 | 70 | 125 | 189 | 75 | 219 |
| /v1/store | 1037 | 0 | 68 | 148 | 251 | 77 | 327 |
| /v1/update | 987 | 0 | 171 | 437 | 641 | 202 | 874 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

