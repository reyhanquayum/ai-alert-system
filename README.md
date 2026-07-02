# ai-alert-system

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
