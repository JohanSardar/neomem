---
name: neomem-control
description: Build and verify neomem features with neomemctl and the feature graph. Use when picking the next feature, writing a proof scenario, proving a feature, benchmarking, or checking that the graph matches the code.
---

# neomem-control

`neomemctl` is the dev-only control CLI. It always runs an isolated neomem (its own temporary `NEOMEM_HOME`, socket and data). It never touches the real `~/.neomem`.

## The loop for every feature

1. `neomemctl map --next` picks the first feature whose dependencies are all proven.
2. If the feature adds a module or changes an interface, design the interface first: a deep module with a small surface, and seams only where SPEC.md §13.3 lists them.
3. Write the proof scenario first in `crates/neomem-control/src/scenarios/<proof>.rs`, register it in `scenarios::ALL`, and watch `neomemctl prove <id>` fail.
4. Build until `cargo nextest run`, `cargo clippy --workspace --all-targets -- -D warnings` and `neomemctl prove <id>` all pass.
5. Clean up the diff: delete dead code, sharpen names, keep one paved path.
6. `prove` is the only thing that changes a feature's status. Never edit `status` or `last_proof` by hand.

## Commands

| Command | Use |
|---|---|
| `neomemctl map [--status]` | every feature, its status, `→` for unblocked |
| `neomemctl map --next` | the next feature to build |
| `neomemctl map --check` | graph versus code drift (also run by `cargo xtask check`) |
| `neomemctl prove <id>\|--changed\|--all` | run proof scenarios; bundles land in `proofs/<id>/<time>/` |

## Proof bundles

`proof.json` holds pass or fail, every claim with its detail, timings, the model mode and the code version. `exchanges.jsonl` holds the requests and responses, and `daemon.log` the daemon's log. Only the newest bundle per feature stays in the tree; git keeps the rest.

A claim is evidence, not an opinion: "search p95 under 30 ms", "every turn captured exactly once", "a query worded like an old fact returns the newest version".

## Rules

- No comments in Rust, anywhere. Names and types explain; prose goes in `docs/`.
- No `unwrap` outside tests, no `unsafe` outside the llama.cpp bindings, no delete method on any store.
- When the spec is silent, choose the simplest fit and add one line to `docs/decisions.md`.
- A live check that needs the user (a browser sign-in, a missing key) gets `blocked: needs-user: <why>` on its feature, and the feature is proven against the fakes.
