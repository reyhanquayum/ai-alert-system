# ai-alert-system

[![CI](https://github.com/reyhanquayum/ai-alert-system/actions/workflows/ci.yml/badge.svg)](https://github.com/reyhanquayum/ai-alert-system/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Autonomous AI incident responder, in Rust. When a production alert fires it:

1. identifies the likely bad commit (`git log` + Claude),
2. finds the most relevant runbook and extracts first steps,
3. estimates user impact from a metrics snapshot,
4. posts a brief to Slack,
5. and auto-generates a markdown postmortem when the incident is resolved.

Design rationale and architecture: see [plan.md](plan.md).

## Quick start (no keys needed — mock LLM backend)

```sh
# build a demo git repo with a plausible "bad commit"
scripts/make_demo_repo.sh /tmp/aas-demo-repo

# fire a sample alert through the full pipeline and auto-resolve it
cargo run -- simulate --sample high-error-rate --repo /tmp/aas-demo-repo \
    --resolve "Rolled back the checkout retry refactor"
```

Samples: `high-error-rate`, `db-connections` (Alertmanager-format payload), `high-latency`.

<details>
<summary><b>Example output</b> — the Slack brief and auto-generated postmortem for the checkout alert</summary>

```
[slack brief]
:rotating_light: *INC-20260702035451-76f2: CheckoutHighErrorRate* (severity: critical)
*Service:* `checkout-api`
*What's happening:* 5xx error rate on checkout-api is above 20% for 5 minutes
*Likely cause:* `6c4b620bf1` — Refactor checkout retry logic and remove timeout guard
on payment calls (Jun Park) — confidence: high
> Commit message and changed files share 4 keyword(s) with the alert, making it the
> strongest candidate in the lookback window.
*Impact:* High — 24% of requests to checkout-api are failing (~60k affected requests/hour)
*Runbook:* Elevated 5xx Error Rate (`runbooks/high-error-rate.md`)
*Next steps:*
1. Check the deploy dashboard: did a deploy land within the alert window? If yes,
   roll it back first and ask questions later.
2. Inspect error logs for the failing service and identify the dominant error signature.
3. Check upstream dependencies (payment gateway, database, cache) for their own alerts.
4. If a retry storm is amplifying load, enable the circuit breaker or reduce client
   retry budgets.
_Status: investigating — replies in thread please._
```

```markdown
# Postmortem: INC-20260702035451-76f2 — CheckoutHighErrorRate

## Summary
An alert (`CheckoutHighErrorRate`) fired for service `checkout-api`. ...

## Timeline
- 03:54:51Z — alert received; incident opened
- 03:54:51Z — suspect commit identified: 6c4b620b (high)
- 03:54:51Z — runbook matched: runbooks/high-error-rate.md
- 03:54:51Z — impact estimated
- 03:54:51Z — brief posted to Slack
- 03:54:51Z — incident resolved: Rolled back the checkout retry refactor

## Root Cause
Suspected commit `6c4b620b...` ("Refactor checkout retry logic and remove timeout
guard on payment calls", by Jun Park) ...

## Impact | What Went Well | What Went Poorly | Action Items
...
```

</details>

## Real mode

```sh
export ANTHROPIC_API_KEY=sk-ant-...          # enables the Claude backend (claude-opus-4-8)
export SLACK_WEBHOOK_URL=https://hooks.slack.com/services/...   # optional; dry-run log otherwise
cargo run -- serve
```

Then point Alertmanager (or curl) at it:

```sh
curl -s -X POST localhost:8080/alerts -H 'content-type: application/json' \
  -d @examples/alerts/high-error-rate.json
# -> {"incidents":["INC-..."]}  (investigation continues in the background)

curl -s localhost:8080/incidents
curl -s localhost:8080/incidents/INC-.../resolve -X POST \
  -H 'content-type: application/json' -d '{"note":"rolled back deploy"}'
```

## Configuration

`config.toml` (all optional — see comments in the file). Point `[repo] path`
at the repository of the service that pages you, and drop your runbooks in
`runbooks/*.md`. Incidents persist to `data/incidents/`, postmortems to
`postmortems/`.

## Tests

```sh
cargo test
```
