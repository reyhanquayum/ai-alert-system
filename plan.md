# AI Alert System — Plan & Design

An autonomous AI incident responder: the moment a production alert fires, it
identifies the likely bad commit, finds the right runbook, estimates user
impact, posts a Slack brief, and auto-generates a postmortem once the incident
is resolved.

## 1. Language choice

**Rust.** Reasoning:

| Criterion | Rust | C++ | TypeScript |
|---|---|---|---|
| Long-running daemon reliability | Excellent (no GC pauses, memory safety, `Result`-driven error handling) | Good but easy to shoot yourself | Good |
| Async HTTP server + client ecosystem | Mature (tokio, axum, reqwest, serde) | Fragmented (boost.beast, cpr…) — much more boilerplate | Mature (express/fastify) |
| Anthropic SDK | None official — raw HTTP (trivial: one JSON endpoint) | None | Official SDK |
| Deployment | Single static binary | Binary + toolchain pain | Node runtime + node_modules |
| Fit with your preference | ✅ | ✅ | — |

TypeScript would only win on the official SDK, but the Claude Messages API is
a single JSON-over-HTTP endpoint — reqwest + serde cover it in ~100 lines.
For an always-on responder that must not fall over during an outage, Rust's
reliability story plus your stated preference makes it the clear pick. C++
would work but costs far more boilerplate for HTTP/JSON/async with no upside.

## 2. High-level architecture

```
 Alertmanager / Grafana / curl
            │  POST /alerts (webhook)
            ▼
 ┌─────────────────────────────────────────────────┐
 │  ai-alert-system (axum HTTP server or CLI)      │
 │                                                 │
 │  Alert normalizer ──► Incident store (JSON)     │
 │                        │                        │
 │        ┌───────────────┼────────────────┐       │
 │        ▼ (concurrent)  ▼                ▼       │
 │  Suspect-commit   Runbook match   Impact est.   │
 │  (git log + LLM)  (index + LLM)   (metrics+LLM) │
 │        └───────────────┼────────────────┘       │
 │                        ▼                        │
 │                 Slack brief (LLM)  ──► Slack    │
 │                                                 │
 │  POST /incidents/{id}/resolve                   │
 │        └──► Postmortem generator (LLM) ──► .md  │
 └─────────────────────────────────────────────────┘
            │ LLM calls: Claude Messages API
            ▼   (claude-opus-4-8, adaptive thinking,
 api.anthropic.com   raw HTTP via reqwest)
```

## 3. Components (crate modules)

| Module | Responsibility |
|---|---|
| `main.rs` | CLI (clap): `serve`, `simulate`, `resolve`, `list` |
| `config.rs` | TOML config + env overrides (`ANTHROPIC_API_KEY`, `SLACK_WEBHOOK_URL`) |
| `alert.rs` | Normalize incoming alerts (Prometheus Alertmanager payload or generic JSON) into one `Alert` type |
| `incident.rs` | Incident lifecycle + JSON file store (`data/incidents/*.json`), timeline tracking |
| `pipeline.rs` | Orchestrator: runs the three analyses **concurrently**, composes the brief, posts to Slack, handles resolution |
| `commits.rs` | `git log` (shell-out, no libgit2 dep) over a configured repo → candidate commits → LLM ranks the likely culprit |
| `runbooks.rs` | Index markdown runbooks, keyword pre-filter, LLM picks the best match + extracts first steps |
| `impact.rs` | Metrics snapshot (mock provider; Prometheus is the extension point) → LLM writes user-impact estimate |
| `slack.rs` | Post brief / resolution to a Slack incoming webhook (dry-run prints to log when unset) |
| `postmortem.rs` | On resolve: gather full incident record → LLM writes a markdown postmortem → `postmortems/INC-….md` |
| `llm.rs` | Claude Messages API client (reqwest, raw HTTP). Two backends: **real** (needs `ANTHROPIC_API_KEY`) and **mock** (deterministic heuristics — same JSON shapes, zero cost, used for tests/demo) |

## 4. LLM integration details

- Endpoint: `POST https://api.anthropic.com/v1/messages`
- Headers: `x-api-key`, `anthropic-version: 2023-06-01`, `content-type: application/json`
- Model: `claude-opus-4-8` (configurable), `thinking: {"type": "adaptive"}`,
  `max_tokens: 8000` (non-streaming is safe below ~16K)
- Five task types, each with its own system prompt:
  1. **SuspectCommit** — input: alert + recent commits (msg, author, files); output JSON `{hash, confidence, reasoning}`
  2. **SelectRunbook** — input: alert + candidate runbooks (pre-scored by keyword overlap); output JSON `{path, reasoning, key_steps[]}`
  3. **EstimateImpact** — input: alert + metrics snapshot; output JSON `{severity_assessment, affected_users, narrative}`
  4. **ComposeBrief** — input: everything above; output: Slack-markdown brief
  5. **Postmortem** — input: full incident record + resolution note; output: markdown document
- JSON tasks instruct the model to reply with a single JSON object; the client
  extracts/parses it. Text blocks are filtered from the `content` array;
  `stop_reason: "refusal"` is surfaced as an error.
- **Mode auto-detection**: real backend when `ANTHROPIC_API_KEY` is set,
  mock otherwise (overridable via `[llm] mode = "real" | "mock" | "auto"`).

## 5. Data flow

1. **Alert fires** → `POST /alerts` (or `simulate` command). Payload normalized; incident `INC-<ts>-<hex>` created and persisted (`status: investigating`).
2. **Investigation** — three analyses run concurrently via `tokio::join!`; each failure is logged but doesn't abort the others (graceful degradation).
3. **Brief** — LLM composes a Slack brief from whatever the analyses produced; posted to the webhook (or logged in dry-run). Incident record updated with all findings + timeline entries.
4. **Resolution** — `POST /incidents/{id}/resolve` (or `resolve` command) with a note → LLM writes the postmortem → saved to `postmortems/` → resolution message posted to Slack.

## 6. HTTP API

| Route | Purpose |
|---|---|
| `POST /alerts` | Ingest alert(s); returns incident IDs (202, investigation continues in background) |
| `GET /incidents` | List incidents |
| `GET /incidents/{id}` | Full incident record |
| `POST /incidents/{id}/resolve` | Body `{"note": "…"}`; triggers postmortem |
| `GET /health` | Liveness |

## 7. Repo layout

```
├── plan.md               ← this file
├── Cargo.toml
├── config.toml           ← default config (committed; env vars override secrets)
├── src/…                 ← modules above
├── runbooks/*.md         ← example runbooks (indexable corpus)
├── examples/alerts/*.json← sample alert payloads for `simulate`
├── scripts/make_demo_repo.sh ← builds a throwaway git repo with a "bad commit"
├── data/incidents/       ← incident store (created at runtime)
└── postmortems/          ← generated postmortems (created at runtime)
```

## 8. Milestones

1. ✅ Scaffolding: Cargo project, config, alert normalization, incident store
2. ✅ LLM client (real + mock backends)
3. ✅ Analyses: suspect commit, runbook match, impact estimate
4. ✅ Pipeline + Slack brief + HTTP server
5. ✅ Resolution flow + postmortem generation
6. ✅ Unit tests + end-to-end demo (`simulate` against a generated demo repo)

## 9. Future work (not in v1)

- Prometheus/Datadog metrics provider (the `impact` module's provider is the seam)
- Slack interactivity (buttons: acknowledge / resolve from Slack)
- Deploy-event correlation (map alert window to deploy SHA range instead of `git log --since`)
- Postmortem action-item tracking (file GitHub issues automatically)
- Streaming LLM responses + retry/backoff on 429/529
