# Load Test Report — 2026-03-08 01:04:47 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 100 clients |
| R/W ratio | 20/80 |
| Mode | rate-limited |
| Duration | 30.1s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 10000 |
| Throughput | 332.7 ops/sec |
| Error rate | 94.26% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 44 | 6 | 149 | 347 | 409 | 168 | 409 |
| /v1/delete | 44 | 0 | 83 | 263 | 282 | 99 | 282 |
| /v1/discover | 500 | 500 | 0 | 2 | 8 | 0 | 41 |
| /v1/export | 500 | 500 | 0 | 8 | 401 | 14 | 479 |
| /v1/recall | 500 | 302 | 0 | 151 | 266 | 152 | 30043 |
| /v1/search | 500 | 301 | 0 | 186 | 308 | 109 | 30026 |
| /v1/store | 7868 | 7817 | 0 | 2 | 128 | 214 | 30047 |
| /v1/update | 44 | 0 | 143 | 344 | 346 | 155 | 346 |

## Fly.io Tier Recommendation

Launch (performance-2x)

Requires Fly Postgres Standard-4 (200 connection limit)

