# Local models

Embeddings always run locally, and the judge and checker fall back to a local reranker (SPEC §10.3, §12, §13.4). Both run in-process through llama.cpp (`llama-cpp-2` 0.1.160).

## Files

`neomemctl fetch-models` puts these in the main checkout's gitignored `models/` (sha256-checked; a second Hugging Face repo is the mirror). An isolated home's `models/` links there; a real home keeps them in `~/.neomem/models/`.

| File | Role | Source | Dims |
|---|---|---|---|
| `embeddinggemma-300m-Q8_0.gguf` | embedder (default) | `ggml-org/embeddinggemma-300M-GGUF` | 768 |
| `qwen3-embedding-0.6b-Q8_0.gguf` | embedder (alternative) | `Qwen/Qwen3-Embedding-0.6B-GGUF` | 1024 |
| `qwen3-reranker-0.6b-Q8_0.gguf` | judge and checker fallback | `ggml-org/Qwen3-Reranker-0.6B-Q8_0-GGUF` | — |

```toml
[models]
embedder = { runtime = "llama.cpp", file = "embeddinggemma-300m-Q8_0.gguf" }
judge    = { provider = "jev", fallback = { runtime = "llama.cpp", file = "qwen3-reranker-0.6b-Q8_0.gguf" } }
checker  = { provider = "jev", fallback = { runtime = "llama.cpp", file = "qwen3-reranker-0.6b-Q8_0.gguf" } }
```

## Backends

| Machine | Backend | How |
|---|---|---|
| macOS | Metal | always on (target-specific dependency) |
| Linux with `nvidia-smi` and `nvcc` | CUDA | `--features neomemd/cuda` |
| Linux with `vulkaninfo`, `glslc` and the Vulkan headers | Vulkan | `--features neomemd/vulkan` |
| anything else | CPU | plain `cargo build` |

`neomemctl` (prove, up, bench) picks the backend itself; `NEOMEM_GPU=cpu|vulkan|cuda` overrides it. A Vulkan build also needs the SPIR-V headers (`apt install spirv-headers`); when they are missing, `neomemctl` installs them once into `~/.cache/neomem/`. A plain `cargo build` is CPU-only (Metal on macOS), so it builds on any machine. When the build has a GPU backend, every layer of every local model goes to the GPU.

## Benchmark

`neomemctl bench models` on a release build: a single 16–32 token query (30 distinct queries, through `neomem models test embedder`, timed inside the daemon), a query plus 16 documents, and the reranker deciding between two options and scoring 15 candidates. §13.5 budgets 5–20 ms for the query embedding (measured on Metal) and ~0.1–0.3 s for the local judge.

Intel Core Ultra 9 288V with Arc 140V (Lunar Lake), 32 GB, Ubuntu, 2026-10-20. The machine was running other builds at the same time (load average 7–9), which hurts the CPU numbers most.

| | CPU (x86-64-v3, 4 threads) p50 / p95 | Vulkan (Arc 140V) p50 / p95 |
|---|---|---|
| EmbeddingGemma-300m, one query | 39.1 / 61.1 ms | 32.6 / 71.8 ms |
| EmbeddingGemma-300m, query + 16 documents | 648 / 727 ms | 230 / 630 ms |
| Qwen3-Embedding-0.6B, one query | 224 / 270 ms | 103 / 140 ms |
| Qwen3-Embedding-0.6B, query + 16 documents | 2398 / 2518 ms | 855 / 2949 ms |
| Qwen3-Reranker-0.6B, decide between 2 options | 336 / 341 ms | 205 / 213 ms |
| Qwen3-Reranker-0.6B, score 15 candidates | 1978 / 1983 ms | 383 / 1619 ms |

On the same machine without other load, an EmbeddingGemma query took 16 ms at 20 tokens and 23 ms at 29 tokens on the CPU, and 25–30 ms on Vulkan at any length from 10 tokens up (small batches pay the iGPU's dispatch cost). The proofs measured the reranker's two-option decision at 64 ms on Vulkan and 295 ms on the CPU.

What follows:
- EmbeddingGemma-300m stays the default embedder: 3–7x faster than Qwen3-Embedding-0.6B for the same retrieval behaviour on the probe pairs (related 0.79 vs unrelated 0.10 for EmbeddingGemma; 0.60 vs 0.12 for Qwen3). Repeated queries skip the model through the query cache (0.02–0.1 ms).
- On this laptop the 20 ms query budget holds on an idle CPU but not under load; the Vulkan build is steadier under CPU load and 3–5x faster for documents and the reranker, so `neomemctl` builds Vulkan here.
- The reranker fits the 0.1–0.3 s fallback budget on Vulkan for gate-sized calls; scoring 15 candidates takes ~0.4 s.

## Idle unloading

Local model weights and contexts load on the first request that needs them. `neomem models` shows `loaded` or `unloaded` for the embedder and reranker. The daemon releases both after `[models] idle_unload` (default `"15m"`) without a query or recall. The next request reloads transparently, including a previously cached query. Document embeddings from background work do not extend the read idle window. The maintenance tick uses the engine clock, so isolated proofs can advance time without waiting fifteen minutes.

## Swapping assignments

`neomem models use planner new-model` keeps the role's provider. `neomem models use planner openai/new-model` selects a configured provider, and `neomem models use embedder qwen3-embedding-0.6b-Q8_0.gguf` selects a local file. Direct config edits also take effect. Other roles use the new assignment on their next call.

An embedder change rebuilds fact and entity vectors in background batches while searches keep using the previous model and snapshot. The daemon publishes the complete replacement snapshot in one step, then removes the old vector rows from the rebuildable index cache. New or edited memories invalidate their staged vectors before activation. A restart retains completed batches and resumes the remaining work. `neomem models` shows the desired assignment; `neomem status` reports the embedding model currently used by the index.
