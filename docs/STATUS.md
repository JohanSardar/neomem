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

`neomemctl bench latency` on a release Vulkan build (Intel Core Ultra 9 288V, Arc 140V iGPU). The search rows come from two runs after the F16 embedder change, on battery power with other work running; the other rows are the idle, plugged-in numbers in `benches/baseline.json`.

| What | p50 | p95 | Budget | Met |
|---|---:|---:|---|---|
| Client start + status (`neomem status`) | 1.2 ms | 2.3 ms | 2–5 ms | yes |
| Unix socket round trip | 0.02 ms | 0.06 ms | < 0.5 ms | yes |
| Capture hook end to end | 1.0 ms | 1.9 ms | < 50 ms | yes |
| `neomem add` end to end | 1.8 ms | 3.9 ms | < 50 ms | yes |
| Search over the socket, 13 facts | 21.9–24.6 ms | 24.6–28.9 ms | 10–30 ms | yes |
| `neomem search` end to end, 13 facts | 23.5–25.8 ms | 27.8–30.6 ms | 10–30 ms | p50 yes, p95 at the limit |
| Search over the socket, 100K facts | 24.6–25.2 ms | 28.1–28.4 ms | 10–30 ms | yes |
| `neomem search` end to end, 100K facts | 26.4–28.5 ms | 31.6–32.3 ms | 10–30 ms | p50 yes, p95 no |
| MCP search over stdio | 20.4–24.4 ms | 25.5–28.2 ms | 10–30 ms | yes |
| Recall, fake models | 3.8 ms | 5.2 ms | < 3.5 s | yes |
| Recall gate, fake models | 0.7 ms | 1.3 ms | ~0.15–0.3 s | yes |

Query embedding is almost the whole search: ~20 ms on this laptop's Arc iGPU since the embedder runs from F16 weights (it was ~29 ms with Q8_0; the spec budgets 5–20 ms on Metal). Everything else in a search at 100K facts takes ~0.8 ms (HNSW seeds 0.6–0.95 ms, latest swap 0.1 ms, ranking and one hop < 0.03 ms). `neomem search` end to end at 100K facts still misses at p95 by 1.6–2.3 ms in these battery-powered runs, where starting the client costs 2–4 ms instead of 1.2 ms. Repeated queries hit the query cache (< 0.01 ms). Live Jev answers the gate in ~100 ms (p50).

Perf rounds after the features were proven:

| Round | Hotspot (profiled) | Change | Result |
|---|---|---|---|
| 1 | Daemon start at 100K facts: HNSW built one vector at a time (36.7 s) | parallel bulk insert (#62) | 14.0 s, −62% |
| 2 | ~9% of search CPU in llama.cpp's per-query Vulkan graph optimization | `GGML_VK_DISABLE_GRAPH_OPTIMIZE` (#63); flash attention off tried (−1.3%, dropped) | query embedding −6% |
| 3 | Non-embedding search pipeline at 100K | none possible: 0.8 ms of ~27 ms | < 3% |
| 4 | Capture hook with 0–200 appended turns | none: 0.8–1.8 ms, process start dominates | < 3% |
| 5 | Recall | none: fake-model time is the model calls; real time is the planner and Jev | < 3% |
| 6 | Query embedding | EmbeddingGemma's QAT Q4_0 instead of Q8_0, tried and not adopted | search −17% end to end at 100K facts, same answers on 60 labeled queries, but Google's MTEB scores drop (code 68.70 → 67.99, English 69.49 → 69.31) and SPEC §13.4 names Q8_0 |
| 7 | Query embedding: llama.cpp's Vulkan Q8_0 matrix multiply (24 of 31 ms at 16 tokens), plus GPU buffers growing on the first query longer than any before (20–75 ms) | F16 copy of the same Q8_0 weights and a 512-token warm-up at load (#67) | search p95 −20 to −40% (MCP 36–39 → 25–28 ms, socket at 100K 38–48 → 28 ms) |

Earlier perf work during the build: the fake model server's Nagle stall (47 → 2.7 ms per call), llama.cpp optimized in debug builds (566 → 38 ms per embedding), and background writes reloading the embedder for every batch (#54).

## Evaluation (§16)

Details, settings and reproduction are in `docs/eval.md`. All model work ran locally (Qwen3-4B Q4_K_M through llama.cpp on the Arc GPU for extraction, consolidation, reading and answer grading; embeddinggemma-300m for embeddings) with the live Jev judge, because no hosted LLM key is available.

| Set | Questions | memory-off | neomem search | Retrieval (recall@5) |
|---|---:|---:|---:|---|
| LongMemEval-S, development sample (one per type) | 7 | 1/7 | 3/7 | knowledge-update 50%, multi-session 100%, single-session-user 100%, temporal 50%, preference 0%, assistant 0% |
| Own coding set (this project's Claude Code and Codex sessions) | 50 | 0/50 | 8/50 | build 12%, capture 35%, models 50%, update 100% |

The first baseline found almost no facts; following it up found and fixed five extraction problems (PRs #54, #55, #57, #59, #60). These are small development samples on a 4B local model, not comparable to published scores; the held-out questions were not scored.

## Needs you

- **Live LLM providers** (`models.llm` is proven against fakes for every kind): `neomem login chatgpt` needs your browser sign-in; OpenAI and Anthropic need API keys in `~/.neomem/secrets.env`; neomem never uses Codex's own login. The spec's default extractor and consolidator are hosted models, so the eval used a local Qwen3-4B instead; a run with a hosted extractor needs one of these.
- **macOS.** The launchd agent, Metal and the macOS CI job are written but untested: this machine is Linux.
- **An idle, plugged-in bench.** The search rows above were measured on battery. `neomemctl bench latency --save-baseline` on AC power with nothing else running records the new baseline for every row.

## Incidents

- An eval probe started a daemon on a temporary home without `[capture]` folders. It read your real `~/.claude`, `~/.codex` and `~/.pi` sessions into that temporary home (36,320 records) before the eval worker stopped it and deleted the home; no model ran on them and nothing left the machine. Since PR #56 only `~/.neomem` falls back to the default agent folders.

## Where to look

- `SPEC.md` is the specification; `docs/decisions.md` lists every gap filled and every deviation.
- `CONTRACT.md` for other systems; `docs/models.md` for model roles, GPU backends and model numbers; `docs/eval.md` for the evaluation.
- `neomemctl map` shows the feature graph; `neomemctl prove <id>` re-runs a proof; `neomemctl bench latency` re-measures speed.
