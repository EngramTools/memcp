# Load Test Report — 2026-03-08 00:44:38 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 10 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 8.8s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 1000 |
| Throughput | 114.2 ops/sec |
| Error rate | 26.00% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 118 | 0 | 23 | 106 | 125 | 34 | 260 |
| /v1/delete | 118 | 0 | 26 | 88 | 105 | 32 | 111 |
| /v1/discover | 130 | 130 | 1 | 7 | 9 | 1 | 14 |
| /v1/export | 130 | 130 | 335 | 538 | 589 | 353 | 657 |
| /v1/recall | 120 | 0 | 91 | 170 | 256 | 97 | 271 |
| /v1/search | 120 | 0 | 57 | 131 | 252 | 66 | 278 |
| /v1/store | 137 | 0 | 24 | 88 | 104 | 35 | 219 |
| /v1/update | 127 | 0 | 32 | 97 | 238 | 44 | 263 |

## Fly.io Tier Recommendation

Starter (shared-cpu-2x)

Requires Fly Postgres Basic (50 connection limit)

