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
