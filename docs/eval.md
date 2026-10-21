# Evaluation

The `neomem-eval` binary measures the real daemon through `neomem-proto`. Each question starts a fresh temporary `NEOMEM_HOME`, adds all dated history turns through `Add` (or replays sanitized coding transcript windows through `Capture`), waits for extraction jobs to finish, awaits `Dream`, then uses `Search`. Failed extraction jobs abort the run. Capture roots stay inside the temporary home and HTTP is disabled. Intake debounce is one second; other daemon model settings come from the run manifest.

```
source ~/.cargo/env
export CARGO_TARGET_DIR=$PWD/target
cargo build -p neomem-eval -p neomemd
nice -n 10 python3 scripts/eval-baseline.py --out /home/johan/neomem/eval-data/baseline
```

Prerequisites are the private datasets and `models.json` in `/home/johan/neomem/eval-data`, the embedding GGUF in `/home/johan/neomem/models`, and the local OpenAI-compatible server specified in that manifest. The script reads only `JEV_API_KEY` from the environment or `~/.neomem/secrets.env`, without printing or copying it. It runs arms sequentially. `neomem-eval report <run-directory>` regenerates aggregate reports from saved results.

Each run records a dataset SHA-256, selected and held-out question IDs, sampling seed, exact daemon configuration, reader and answer-judge endpoints and versions, model file hashes, prompts, context cap, and k. Questions are ordered by SHA-256(seed:ID) within types and selected round-robin. Development samples reserve a held-out item per type where possible, plus all unselected items. Abstention is a separate stratum.

Recall@k measures the fraction of gold evidence sessions covered by the sources of the first k facts. Precision@5 counts evidence-backed facts among five slots, including unfilled slots as misses. Abstentions and memory-off have no retrieval denominator. Accuracy uses the official LongMemEval answer-check prompts, including its abstention and temporal rules. Reader context contains only returned fact text, capped at 10,000 characters. Context tokens are the provider's actual total reader prompt-token usage. Answer latency includes search and reader completion, excludes offline ingestion and answer grading; ingestion time is recorded separately. No vendor score is directly comparable to this local-model run.

Datasets, transcripts, questions, model manifests, and per-question outputs remain gitignored. Only aggregate baseline results belong here.
