# Load Test Report — 2026-03-08 00:54:39 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 500 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 426.8s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 50000 |
| Throughput | 117.2 ops/sec |
| Error rate | 35.72% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 5829 | 721 | 981 | 30332 | 31266 | 4542 | 61563 |
| /v1/delete | 5835 | 661 | 742 | 30424 | 31807 | 4622 | 62127 |
| /v1/discover | 6500 | 6500 | 6 | 231 | 573 | 45 | 1775 |
| /v1/export | 6500 | 6500 | 1029 | 30215 | 30947 | 4111 | 32017 |
| /v1/recall | 6000 | 838 | 918 | 30492 | 31394 | 5312 | 62703 |
| /v1/search | 6000 | 712 | 496 | 30292 | 30959 | 4096 | 32285 |
| /v1/store | 7034 | 1160 | 785 | 30524 | 31172 | 5992 | 32412 |
| /v1/update | 6302 | 768 | 1091 | 30428 | 31243 | 4880 | 33974 |

## Fly.io Tier Recommendation

Enterprise (performance-8x)

Requires Fly Postgres Standard-16 (1000 connection limit)

