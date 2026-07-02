# Elevated 5xx Error Rate

Applies to: any HTTP API service (checkout-api, orders-api, search-api).

## Symptoms

- Error-rate alert firing (5xx above threshold)
- Users report failed requests, retries, or blank pages
- Possible retry storms amplifying load on upstreams

## Diagnosis and mitigation

1. Check the deploy dashboard: did a deploy land within the alert window? If yes, roll it back first and ask questions later.
2. Inspect error logs for the failing service and identify the dominant error signature (timeout vs. exception vs. dependency failure).
3. Check upstream dependencies (payment gateway, database, cache) for their own alerts.
4. If a retry storm is amplifying load, enable the circuit breaker or reduce client retry budgets.
5. Scale out the service if error rate correlates with CPU/memory saturation.

## Escalation

Page the owning team (see service catalog) if error rate stays above threshold 15 minutes after mitigation.
