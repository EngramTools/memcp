# Load Test Report — 2026-03-08 00:18:51 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 100 clients |
| R/W ratio | 80/20 |
| Mode | raw |
| Duration | 35.1s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 284.9 ops/sec |
| Error rate | 40.42% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 490 | 0 | 87 | 245 | 330 | 102 | 367 |
| /v1/delete | 490 | 1 | 96 | 163 | 250 | 164 | 30002 |
| /v1/discover | 2000 | 2000 | 1 | 7 | 18 | 1 | 34 |
| /v1/export | 2000 | 2000 | 60 | 126 | 174 | 68 | 292 |
| /v1/recall | 2000 | 2 | 107 | 186 | 289 | 145 | 30051 |
| /v1/search | 2000 | 18 | 51 | 96 | 276 | 324 | 30030 |
| /v1/store | 530 | 20 | 98 | 227 | 30025 | 1234 | 30040 |
| /v1/update | 490 | 1 | 155 | 311 | 394 | 234 | 30003 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

