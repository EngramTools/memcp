# Load Test Report — 2026-03-08 00:45:17 UTC

**Git SHA:** `9f65719`

## Configuration

| Parameter | Value |
|-|-|
| Corpus size | 10000 memories |
| Concurrency | 50 clients |
| R/W ratio | 50/50 |
| Mode | raw |
| Duration | 35.8s |

## Overall Results

| Metric | Value |
|-|-|
| Total ops | 5000 |
| Throughput | 139.8 ops/sec |
| Error rate | 26.04% |

## Per-Endpoint Statistics

| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |
|-|-|-|-|-|-|-|-|
| /v1/annotate | 588 | 0 | 279 | 647 | 925 | 315 | 1401 |
| /v1/delete | 588 | 0 | 274 | 551 | 659 | 298 | 1104 |
| /v1/discover | 650 | 650 | 2 | 120 | 297 | 18 | 529 |
| /v1/export | 650 | 650 | 691 | 1061 | 1385 | 724 | 1632 |
| /v1/recall | 600 | 0 | 347 | 787 | 942 | 394 | 1298 |
| /v1/search | 600 | 0 | 211 | 507 | 747 | 244 | 822 |
| /v1/store | 687 | 2 | 260 | 547 | 720 | 363 | 30015 |
| /v1/update | 637 | 0 | 407 | 711 | 914 | 428 | 1139 |

## Fly.io Tier Recommendation

Growth (shared-cpu-4x)

Requires Fly Postgres Standard-2 (100 connection limit)

