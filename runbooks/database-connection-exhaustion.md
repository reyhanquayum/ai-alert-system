# Database Connection Pool Exhaustion

Applies to: postgres-backed services (orders-db, users-db).

## Symptoms

- Connection pool saturated; queries queueing or timing out
- `too many connections` or pool-acquire timeout errors in service logs
- Often caused by a connection leak in a recent change or a slow-query pileup

## Diagnosis and mitigation

1. Check pool metrics: active vs. idle connections, acquire wait time.
2. Run `pg_stat_activity` to find long-running or idle-in-transaction queries; terminate offenders.
3. Check whether a recent deploy changed pool sizing, transaction scope, or added an unclosed connection path — roll back if so.
4. As a stopgap, restart the worst-offending service pods to release leaked connections.
5. If load is legitimate, raise the pool ceiling within the database's max_connections budget.

## Escalation

Involve the DBA on-call if the database itself (not the pool) is the bottleneck.
