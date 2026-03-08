# Load Test Report — 2026-03-08 00:16:05 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 100 memories |
| Concurrency | 100 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 35.1s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 284.6 ops/sec |
| Error rate | 40.38% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 490 | 0 | 103 | 561 | 1005 | 235 | 30069 |
| /v1/delete | 490 | 3 | 51 | 136 | 184 | 302 | 30065 |
| /v1/discover | 2000 | 2000 | 0 | 2 | 28 | 1 | 221 |
| /v1/export | 2000 | 2000 | 12 | 40 | 124 | 76 | 30022 |
| /v1/recall | 2000 | 11 | 49 | 129 | 317 | 223 | 30085 |
| /v1/search | 2000 | 10 | 19 | 60 | 171 | 174 | 30031 |
| /v1/store | 530 | 12 | 55 | 292 | 30029 | 805 | 30067 |
| /v1/update | 490 | 2 | 185 | 706 | 1251 | 372 | 30155 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

