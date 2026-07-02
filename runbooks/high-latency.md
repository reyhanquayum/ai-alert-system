# High Latency / Slow Responses

Applies to: latency-sensitive APIs (search-api, checkout-api).

## Symptoms

- p95/p99 latency above SLO
- Cache hit rate drops, upstream fan-out slow, or GC/CPU pressure

## Diagnosis and mitigation

1. Compare latency against the deploy timeline; roll back any deploy inside the regression window.
2. Check cache hit rate — a drop usually means a cache key change or cold cache after deploy; warm or revert.
3. Profile the slowest endpoint (tracing dashboard) and identify the slow span: database, upstream call, or compute.
4. Check for noisy neighbors: co-located batch jobs or a traffic spike from a single client.
5. Scale horizontally if saturation is genuine and no regression is found.

## Escalation

Escalate to the performance guild if latency stays degraded after rollback and scaling.
