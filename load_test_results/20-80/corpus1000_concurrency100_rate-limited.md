# Load Test Report — 2026-03-08 01:01:18 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 1000 memories |
| Concurrency | 100 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 30.4s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 329.2 ops/sec |
| Error rate | 93.51% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 63 | 10 | 112 | 220 | 30089 | 1068 | 30089 |
| /v1/delete | 64 | 1 | 83 | 149 | 30006 | 555 | 30006 |
| /v1/discover | 500 | 500 | 0 | 1 | 3 | 0 | 5 |
| /v1/export | 500 | 500 | 0 | 2 | 77 | 3 | 262 |
| /v1/recall | 500 | 302 | 1 | 153 | 263 | 155 | 30175 |
| /v1/search | 500 | 301 | 1 | 102 | 216 | 80 | 30103 |
| /v1/store | 7810 | 7736 | 0 | 1 | 226 | 140 | 30180 |
| /v1/update | 63 | 1 | 102 | 159 | 190 | 107 | 190 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

