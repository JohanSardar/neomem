# neomem status

What was built from SPEC.md, how fast it is against the §13.5 budget, how it scores on the §16 baseline, and what still needs you.

## Built

Every feature in `.agents/skills/neomem-control/feature-graph.yaml` is proven: `neomemctl prove --all` passes on main (35 of 35 at `68849fb`). Each proof runs a real `neomemd` in an isolated home and checks its claims through the same surfaces a user or agent uses.

| Area | Features |
|---|---|
| Daemon and intake | `daemon` (socket, HTTP, protocol version, start on demand), `intake`, `spool` (writes wait while the daemon is down), `raw` (keyword search over the intake log; `search` falls back to it when the daemon cannot start), `correct` |
| Storage | `vault` (Markdown + YAML, atomic writes, local git), `index` (index.db and the in-memory snapshot), `drift` (nightly repair) |
| Models | `models.embed` (llama.cpp on Vulkan/CUDA/CPU/Metal, or any OpenAI-compatible endpoint), `models.judge` (Jev with a local reranker fallback; live Jev verified), `models.llm` (openai, anthropic, chatgpt, codex, command), `models.swap`, `models.idle` |
| Write path | `extract` (batching, one call per batch, owner-aware), `check` (same / extends / updates / contradicts / new) |
| Search | `search`, `latest`, `links`, `history`, `entity`, `decay` |
| Consolidation | `reconcile` (fact reconciler), `entities` (entity keeper), `neomem dream` and its triggers |
| Agents | `capture.cc`, `capture.codex`, `capture.pi`, `recall.gate`, `recall.plan`, `recall.approve`, `recall.hooks`, `mcp`, `skill`, `install` |
| Other systems | `contract` (CONTRACT.md + JSON Schema, tags, external links) |
| Evaluation | `eval` (`neomem-eval`, LongMemEval-S and the own coding set) |

`neomem install` wires Claude Code (plugin with hooks and MCP), Codex (plugin, MCP, its hooks trusted), pi (the `neomem-pi` extension with capture, recall and neomem's read tools), the skill for all three, and a systemd user unit (a launchd agent on macOS). Its proof installs into a sandboxed home with a private systemd user manager, then runs a real headless `claude -p`, `codex exec` and `pi -p` session against the fake model service: each session is captured, sees the skill and the neomem tools, and gets the recalled fact with its `fct_` ID in its model call; pi also answers from a `neomem_search` tool call.

## Speed against the §13.5 budget

`neomemctl bench latency` on a release Vulkan build, idle machine (Intel Core Ultra 9 288V, Arc 140V iGPU); the numbers are in `benches/baseline.json`.

| What | p50 | p95 | Budget | Met |
|---|---:|---:|---|---|
| Client start + status (`neomem status`) | 1.2 ms | 2.3 ms | 2–5 ms | yes |
| Unix socket round trip | 0.02 ms | 0.06 ms | < 0.5 ms | yes |
| Capture hook end to end | 1.0 ms | 1.9 ms | < 50 ms | yes |
| `neomem add` end to end | 1.8 ms | 3.9 ms | < 50 ms | yes |
| Search over the socket, 13 facts | 20.9 ms | 32.0 ms | 10–30 ms | p50 yes, p95 no |
| `neomem search` end to end, 13 facts | 24.5 ms | 34.6 ms | 10–30 ms | p50 yes, p95 no |
| Search over the socket, 100K facts | 28.3 ms | 40.7 ms | 10–30 ms | p50 yes, p95 no |
| `neomem search` end to end, 100K facts | 27.5 ms | 37.6 ms | 10–30 ms | p50 yes, p95 no |
| MCP search over stdio | 25.0 ms | 32.0 ms | 10–30 ms | p50 yes, p95 no |
| Recall, fake models | 3.8 ms | 5.2 ms | < 3.5 s | yes |
| Recall gate, fake models | 0.7 ms | 1.3 ms | ~0.15–0.3 s | yes |

Search misses the budget at p95 for one reason: embedding the query takes ~29 ms on this laptop's Arc iGPU (llama.cpp's own `llama-bench` needs 33 ms for a 24-token pass of the same model), where the spec budgets 5–20 ms on Metal. Everything else in a search at 100K facts takes ~0.8 ms (HNSW seeds 0.6–0.95 ms, latest swap 0.1 ms, ranking and one hop < 0.03 ms). The CPU backend is slower here (35 ms). Repeated queries hit the query cache (< 0.01 ms). Live Jev answers the gate in ~100 ms (p50).

Perf rounds after the features were proven:

| Round | Hotspot (profiled) | Change | Result |
|---|---|---|---|
| 1 | Daemon start at 100K facts: HNSW built one vector at a time (36.7 s) | parallel bulk insert (#62) | 14.0 s, −62% |
| 2 | ~9% of search CPU in llama.cpp's per-query Vulkan graph optimization | `GGML_VK_DISABLE_GRAPH_OPTIMIZE` (#63); flash attention off tried (−1.3%, dropped) | query embedding −6% |
| 3 | Non-embedding search pipeline at 100K | none possible: 0.8 ms of ~27 ms | < 3% |
| 4 | Capture hook with 0–200 appended turns | none: 0.8–1.8 ms, process start dominates | < 3% |
| 5 | Recall | none: fake-model time is the model calls; real time is the planner and Jev | < 3% |

Earlier perf work during the build: the fake model server's Nagle stall (47 → 2.7 ms per call), llama.cpp optimized in debug builds (566 → 38 ms per embedding), and background writes reloading the embedder for every batch (#54).

## Evaluation (§16)

Details, settings and reproduction are in `docs/eval.md`. All model work ran locally (Qwen3-4B Q4_K_M through llama.cpp on the Arc GPU for extraction, consolidation, reading and answer grading; embeddinggemma-300m for embeddings) with the live Jev judge, because no hosted LLM key is available.

| Set | Questions | memory-off | neomem search | Retrieval (recall@5) |
|---|---:|---:|---:|---|
| LongMemEval-S, development sample (one per type) | 7 | 1/7 | 3/7 | knowledge-update 50%, multi-session 100%, single-session-user 100%, temporal 50%, preference 0%, assistant 0% |
| Own coding set (this project's Claude Code and Codex sessions) | 50 | 0/50 | 8/50 | build 12%, capture 35%, models 50%, update 100% |

The first baseline found almost no facts; following it up found and fixed five extraction problems (PRs #54, #55, #57, #59, #60). These are small development samples on a 4B local model, not comparable to published scores; the held-out questions were not scored.

## Needs you

- **GitHub CI.** The `gh` token has no `workflow` scope, so GitHub refuses the workflow file. Run `scripts/enable-ci.sh`: it asks gh for the scope in your browser, moves `ci/workflows/ci.yml` into `.github/workflows/` and pushes. Until then `cargo xtask ci` (format, build, nextest, clippy pedantic, cargo-deny, xtask check, `prove --changed`) is the gate. Every PR passed it; for one (#47) it ran before the last merge of main into the branch, and the same gate on the merged main passed right after.
- **Live LLM providers** (`models.llm` is proven against fakes for every kind): `neomem login chatgpt` needs your browser sign-in; OpenAI and Anthropic need API keys in `~/.neomem/secrets.env`; neomem never uses Codex's own login. The spec's default extractor and consolidator are hosted models, so the eval used a local Qwen3-4B instead; a run with a hosted extractor needs one of these.
- **macOS.** The launchd agent, Metal and the macOS CI job are written but untested: this machine is Linux.

## Incidents

- An eval probe started a daemon on a temporary home without `[capture]` folders. It read your real `~/.claude`, `~/.codex` and `~/.pi` sessions into that temporary home (36,320 records) before the eval worker stopped it and deleted the home; no model ran on them and nothing left the machine. Since PR #56 only `~/.neomem` falls back to the default agent folders.

## Where to look

- `SPEC.md` is the specification; `docs/decisions.md` lists every gap filled and every deviation.
- `CONTRACT.md` for other systems; `docs/models.md` for model roles, GPU backends and model numbers; `docs/eval.md` for the evaluation.
- `neomemctl map` shows the feature graph; `neomemctl prove <id>` re-runs a proof; `neomemctl bench latency` re-measures speed.
