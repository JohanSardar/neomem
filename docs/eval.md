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

Hardware is Ubuntu 26.04.1, Intel Core Ultra 9 288V (8 cores), 30 GB RAM and Intel Arc 140V Vulkan GPU. LongMemEval uses `xiaowu0162/longmemeval-cleaned`, file `longmemeval_s_cleaned.json`, SHA-256 `d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442`.

## What the first run exposed

The first baseline (source `49710e6`) answered only the abstention question in both arms: extraction wrote no facts for six of seven LongMemEval histories and for all fifty coding windows. Proxying the local extractor's requests showed why, and each cause was fixed in the product, not in the evaluation:

- the extractor was never told who speaks in `user:` turns, so it dropped first-person facts or borrowed a name from the prompt's examples (#55);
- it turned assistants' general advice into "the user plans to ..." facts and skipped the user's own details (#57);
- it ignored what an agent reported doing and finding in coding sessions (#59);
- facts citing entities the model forgot to list were rejected, and non-owner entities were filed under the owner (#60);
- background writes reloaded the embedder for every batch, about 0.5 s each (#54).

These rules were designed by inspecting extraction batches from the development sample, which §16.6 allows; the held-out questions were never scored or inspected.

## Runs

LongMemEval: both arms ran at source `329938c` (after #54, #55 and #57) with executables `neomem-eval` `603b0451…c4a155a` and `neomemd` `e47282a3…bc6a647536efa3`; ingestion of the seven search-arm histories took 15,201 s in total. The coding set ran again after #59 and #60 at source `96b2dc3` with `neomemd` `012a1181…3ed695050` and the same `neomem-eval`; its search-arm ingestion took 2,106 s. Write-time generation samples at temperature 0.8, so a replay can extract different facts. Per-question outputs stay private under `eval-data/`.

## LongMemEval-S development sample

Search answers 3/7 correctly, against 1/7 with memory off. One question per type supports no strong generalization.

| Type | Arm | N | Recall@5 | Recall@10 | Precision@5 | Accuracy | Mean answer seconds | Mean prompt tokens |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| abstention | memory-off | 1 | — | — | — | 100.0% | 0.72 | 104.0 |
| knowledge-update | memory-off | 1 | — | — | — | 0.0% | 0.62 | 107.0 |
| multi-session | memory-off | 1 | — | — | — | 0.0% | 0.58 | 107.0 |
| single-session-assistant | memory-off | 1 | — | — | — | 0.0% | 0.60 | 141.0 |
| single-session-preference | memory-off | 1 | — | — | — | 0.0% | 0.68 | 109.0 |
| single-session-user | memory-off | 1 | — | — | — | 0.0% | 0.67 | 101.0 |
| temporal-reasoning | memory-off | 1 | — | — | — | 0.0% | 0.60 | 119.0 |
| abstention | search | 1 | — | — | — | 100.0% | 7.79 | 386.0 |
| knowledge-update | search | 1 | 50.0% | 50.0% | 60.0% | 100.0% | 6.21 | 358.0 |
| multi-session | search | 1 | 100.0% | 100.0% | 100.0% | 100.0% | 8.49 | 376.0 |
| single-session-assistant | search | 1 | 0.0% | 0.0% | 0.0% | 0.0% | 6.46 | 437.0 |
| single-session-preference | search | 1 | 0.0% | 0.0% | 0.0% | 0.0% | 4.45 | 391.0 |
| single-session-user | search | 1 | 100.0% | 100.0% | 100.0% | 0.0% | 21.75 | 248.0 |
| temporal-reasoning | search | 1 | 50.0% | 50.0% | 40.0% | 0.0% | 3.87 | 400.0 |

Search retrieves the gold evidence for the knowledge-update, multi-session, single-session-user and temporal-reasoning questions; the reader still misses the single-session-user and temporal answers. Search returned no evidence for the preference and assistant questions: preference questions reward inferring what the user would like, which neomem deliberately does not do (§3.2), and the assistant question asks about something an assistant said, which the extractor now keeps only when it is a result or decision (#57).

## Private coding development sample

Search answers 8/50 correctly, against 0/50 with memory off; 33 of 50 searches return at least one fact.

| Type | Arm | N | Recall@5 | Recall@10 | Precision@5 | Accuracy | Mean answer seconds | Mean prompt tokens |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| coding-build | memory-off | 17 | — | — | — | 0.0% | 0.64 | 106.2 |
| coding-capture | memory-off | 17 | — | — | — | 0.0% | 0.63 | 106.5 |
| coding-models | memory-off | 14 | — | — | — | 0.0% | 0.63 | 105.1 |
| coding-update | memory-off | 2 | — | — | — | 0.0% | 0.64 | 105.0 |
| coding-build | search | 17 | 11.8% | 11.8% | 2.4% | 11.8% | 2.38 | 155.4 |
| coding-capture | search | 17 | 35.3% | 35.3% | 11.8% | 23.5% | 2.67 | 156.2 |
| coding-models | search | 14 | 50.0% | 50.0% | 18.6% | 14.3% | 2.61 | 141.0 |
| coding-update | search | 2 | 100.0% | 100.0% | 50.0% | 0.0% | 4.40 | 389.0 |

Many answers (exact proof counts, versions) sit in raw tool output, which capture keeps out of memory by design (§5.1), so this set rewards what agents say about their work rather than everything they saw.
