# Evaluation

The `neomem-eval` binary measures the real daemon through `neomem-proto`. Each question starts a fresh temporary `NEOMEM_HOME`, adds all dated history turns through `Add` (or replays sanitized coding transcript windows through `Capture`), waits for extraction jobs to finish, awaits `Dream`, then uses `Search`. Failed extraction jobs abort the run. Capture roots stay inside the temporary home and HTTP is disabled. Intake debounce is one second; other daemon model settings come from the run manifest.

```
source ~/.cargo/env
export CARGO_TARGET_DIR=$PWD/target
cargo build -p neomem-eval -p neomemd --features neomemd/vulkan
nice -n 10 python3 scripts/eval-baseline.py --models crates/neomem-eval/examples/local-models.json --out /home/johan/neomem/eval-data/baseline
```

Prerequisites are the private datasets and `models.json` in `/home/johan/neomem/eval-data`, the embedding GGUF in `/home/johan/neomem/models`, and the local OpenAI-compatible server specified in that manifest. The script reads only `JEV_API_KEY` from the environment or `~/.neomem/secrets.env`, without printing or copying it. It runs arms sequentially. `neomem-eval report <run-directory>` regenerates aggregate reports from saved results.

Each run records a dataset SHA-256, selected and held-out question IDs, sampling seed, exact daemon configuration, reader and answer-judge endpoints and versions, model file hashes, prompts, context cap, and k. Questions are ordered by SHA-256(seed:ID) within types and selected round-robin. Development samples reserve a held-out item per type where possible, plus all unselected items. Abstention is a separate stratum.

Recall@k measures the fraction of gold evidence sessions covered by the sources of the first k facts. Precision@5 counts evidence-backed facts among five slots, including unfilled slots as misses. Abstentions and memory-off have no retrieval denominator. Accuracy uses the official LongMemEval answer-check prompts, including its abstention and temporal rules. Reader context contains only returned fact text, capped at 10,000 characters. Context tokens are the provider's actual total reader prompt-token usage. Answer latency includes search and reader completion, excludes offline ingestion and answer grading; ingestion time is recorded separately. No vendor score is directly comparable to this local-model run.

Datasets, transcripts, questions, model manifests, and per-question outputs remain gitignored. Only aggregate baseline results belong here.

## Local baseline configuration

The baseline uses Qwen3-4B-Instruct-2507 Q4_K_M from `unsloth/Qwen3-4B-Instruct-2507-GGUF`, served by llama.cpp b11780 (0.7.0-dev, commit `1242be66f`) with Vulkan on Intel Arc 140V. One server runs at nice 10 with `--ctx-size 32768 --parallel 1 --threads 4 --n-gpu-layers 99`. The public [model profile](../crates/neomem-eval/examples/local-models.json) records file hashes and sampling settings. Extraction, consolidation, planner, reader and answer judge share this local endpoint. Embedding uses embeddinggemma-300m Q8_0 through llama-cpp-2 0.1.160; write checks and recall judgments use live Jev `jev-1.13.0`. Planner and recall judge are configured but not exercised by these two arms.

Reader and answer judge use temperature 0 and seed 20261021, with output caps 512 and 8 tokens. Write-time generation uses the server defaults: temperature 0.8, random seed, top-k 40, top-p 0.95, min-p 0.05, with output caps 4096 for extraction and 2048 for consolidation. Consequently, replaying the same inputs can change extracted facts. Saved server properties preserve the complete sampler configuration.

The development sample contains seven LongMemEval-S questions, one for each published question type plus abstention, with seed 20261021 and 493 held out. The selected histories comprise 346 sessions and 3506 turns. The initial real ingestion throughput required reducing the requested approximately 60-question sample to this minimum stratified sample to fit the four-hour budget. One item per type supports no strong generalization. Unformatted full histories tokenize to 103190–105338 tokens per selected item, exceeding the local reader's 32768-token context; full-context is therefore skipped.

The private coding corpus contains sanitized material from this project's own permitted Claude Code and Codex sessions. Sixty manually authored questions have checkable transcript answers; fifty are selected with the same seed and ten held out. Each question replays a bounded original transcript window containing its evidence and three fixed distractor events from other permitted project sessions; update questions also include the older event. This measures retrieval over curated windows, not the entire session archive. Replay uses the real Capture protocol and original event timestamps. Questions, source transcripts and per-question outputs remain private under `eval-data/coding-set/`.

The run executes frozen Vulkan binaries, whose SHA-256 values are saved in `binaries.json`. A private source archive and `source.json` preserve the source revision at freeze; each arm's manifest separately records the workspace HEAD at invocation. CI runs on the CPU backend to avoid concurrent GPU contention. Pauses during ingestion for earlier CI runs are recorded in `pauses.jsonl`; wall time and ingestion time include them, while measured answer latency excludes them.

LongMemEval uses `xiaowu0162/longmemeval-cleaned`, file `longmemeval_s_cleaned.json`, SHA-256 `d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442`. Frozen executable SHA-256 values are `eada64bdaf46977e3452da90ec721f7d443d04f9ab6fbe7e834416bf0881f56b` (`neomem-eval`) and `13fc01d40cf9e236b717e5d819033525127fa0ddccf8cfd65cc760e5c567241c` (`neomemd`). The source was frozen at `49710e64aba52d8f5cb03a0b392671b8d3461ebc` before later proof refreshes and main merges; its private source archive has SHA-256 `5fe3d997f502eef3cc0a08e1945eaaf1ffc329bdfc1f14ce5740a9aade1d7098`. Hardware is Ubuntu 26.04.1, Intel Core Ultra 9 288V (8 cores), 30 GB RAM and Intel Arc 140V Vulkan GPU.

The LongMemEval result is a failed effectiveness baseline: both arms answer only the abstention correctly (1/7, 14.3%). Search retrieves no evidence for any of the six answerable questions. Six searches return no facts; the multi-session search returns ten facts with no gold evidence-session sources. This does not establish whether a stronger extractor or larger reader improves results. No prompt, sample or model was tuned against these development answers, and held-out questions were not scored.

## LongMemEval-S development sample

| Type | Arm | N | Recall@5 | Recall@10 | Precision@5 | Accuracy | Mean answer seconds | Mean prompt tokens |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| abstention | memory-off | 1 | — | — | — | 100.0% | 0.85 | 104.0 |
| knowledge-update | memory-off | 1 | — | — | — | 0.0% | 1.04 | 107.0 |
| multi-session | memory-off | 1 | — | — | — | 0.0% | 1.05 | 107.0 |
| single-session-assistant | memory-off | 1 | — | — | — | 0.0% | 1.36 | 141.0 |
| single-session-preference | memory-off | 1 | — | — | — | 0.0% | 1.66 | 109.0 |
| single-session-user | memory-off | 1 | — | — | — | 0.0% | 1.70 | 101.0 |
| temporal-reasoning | memory-off | 1 | — | — | — | 0.0% | 1.80 | 119.0 |
| abstention | search | 1 | — | — | — | 100.0% | 5.45 | 104.0 |
| knowledge-update | search | 1 | 0.0% | 0.0% | 0.0% | 0.0% | 2.95 | 107.0 |
| multi-session | search | 1 | 0.0% | 0.0% | 0.0% | 0.0% | 4.58 | 328.0 |
| single-session-assistant | search | 1 | 0.0% | 0.0% | 0.0% | 0.0% | 6.30 | 141.0 |
| single-session-preference | search | 1 | 0.0% | 0.0% | 0.0% | 0.0% | 5.31 | 109.0 |
| single-session-user | search | 1 | 0.0% | 0.0% | 0.0% | 0.0% | 3.21 | 101.0 |
| temporal-reasoning | search | 1 | 0.0% | 0.0% | 0.0% | 0.0% | 1.77 | 119.0 |

## Private coding development sample

| Type | Arm | N | Recall@5 | Recall@10 | Precision@5 | Accuracy | Mean answer seconds | Mean prompt tokens |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| coding-build | memory-off | 17 | — | — | — | 0.0% | 0.65 | 106.2 |
| coding-capture | memory-off | 17 | — | — | — | 0.0% | 0.63 | 106.5 |
| coding-models | memory-off | 14 | — | — | — | 0.0% | 0.64 | 105.1 |
| coding-update | memory-off | 2 | — | — | — | 0.0% | 0.64 | 105.0 |
| coding-build | search | 17 | 0.0% | 0.0% | 0.0% | 0.0% | 1.68 | 106.2 |
| coding-capture | search | 17 | 0.0% | 0.0% | 0.0% | 0.0% | 1.67 | 106.5 |
| coding-models | search | 14 | 0.0% | 0.0% | 0.0% | 0.0% | 1.67 | 105.1 |
| coding-update | search | 2 | 0.0% | 0.0% | 0.0% | 0.0% | 1.66 | 105.0 |


The complete sequential run took 8620.94 seconds (2 h 23 min 41 s), including 780.99 seconds of recorded ingestion pauses for CI. LongMemEval search ingestion consumed 8044.95 seconds in total. Coding accuracy was 0/50 in both arms, with zero evidence recall and no returned facts in all fifty search queries. These results establish a reproducible low-quality baseline, not useful memory performance. All fifty coding histories were replayed through Capture; no private questions or per-question outputs are committed. A live Jev judge smoke check also succeeded (`jev:jev-1.13.0`, 97.5 ms), clearing the previous missing-key blocker without exposing the key.
