/goal Build neomem, fully working and fast, from ~/neomem/SPEC.md. Work alone until it is done. Never ask me anything.

This machine is an Ubuntu laptop. ~/neomem is a clone of the private repo JohanSardar/neomem, which already has SPEC.md, GOAL.md and diagram/ on main. The spec was written on a Mac, so read "Linux notes" below.

## What you are building
~/neomem/SPEC.md is the only source of truth. Read all of it before writing code, and read it again whenever you are unsure. §13 covers the design and modules, §16 evaluation, and §17 the build method. When the spec does not cover something, pick the simplest option that fits it, record the choice in one line in docs/decisions.md, and keep going.

## Setup (do it yourself, install whatever you need)
- Install anything missing: build-essential, pkg-config, cmake, clang, libssl-dev, sqlite3, rustup stable, cargo-nextest, cargo-deny, cargo-sweep, samply, hyperfine, node/pnpm (for neomem-pi), the gh CLI, and any crates or models the spec names. Use apt (with sudo), cargo, npm and curl installers freely.
- The repo already exists. Work in ~/neomem, pull main, and push branches to origin. Add the AGPL-3.0 LICENSE in the first PR.
- Git identity for this repo only: user.name "JohanSardar", and the GitHub noreply email (`<id>+JohanSardar@users.noreply.github.com`, with the id from `gh api user`). Never use any other email.
- Add a GitHub Actions workflow on push to main that runs `cargo nextest run`, clippy pedantic with `-D warnings`, cargo-deny, `cargo xtask check` and `neomemctl prove --changed`.
- Step 0 comes before anything else (§17): the xtask checks (dependency rules, no-comments lexer, feature-graph check), `neomemctl`, and .agents/skills/neomem-control/ containing feature-graph.yaml. After step 0, follow `neomemctl map --next`.

## How each feature is built (§17 loop, every time)
1. `neomemctl map --next` picks the feature.
2. If it adds or changes an interface, design the interface first (deep module, small surface, the seams the spec lists).
3. Write the proof first and watch it fail.
4. Make it pass. Run cargo test, clippy, and `neomemctl prove <id>`.
5. Clean up: delete dead code, use clear names, follow one paved path.
6. `prove` marks it proven. Never edit a status by hand.

## Code quality bar
- Code must read like clean prose. Names explain everything, because **comments are banned** (the lexer check enforces it). Doc text goes in docs/, not in code.
- Small functions, typed IDs, no `unwrap` outside tests, no `unsafe` outside the llama.cpp bindings, and no delete method anywhere.
- Copy existing patterns in the codebase. Never invent a second way to do something.
- When code looks messy, fix it in the same PR. When a mistake repeats, fix it at the lowest rung of the correction ladder: types first, then a lint, then the skill.

## Commits and PRs: small and often
- One branch per feature slice: `feat/<feature-id>-<slice>`, `perf/...`, `fix/...`, `chore/...`.
- Commits are small and do one thing each (about 20–150 changed lines). Messages are conventional: `feat(index): add f16 hnsw snapshot swap`. Commit every time something compiles and its tests pass.
- Open a PR as soon as a slice works (about 1–5 commits). A PR body has: what it does, the proof command and its output, and bench numbers if speed was touched.
- Merge right away with `gh pr merge --merge --delete-branch` once local checks are green, so the small commits stay in history. Then pull main and start the next slice. Aim for many small PRs per feature, never one giant PR.
- Every PR must be a real, meaningful change. No empty, cosmetic-only or split-for-show PRs.
- If CI on main goes red, the next PR fixes it before any other work.

## Speed (the whole point)
- Budget is in §13.5: search ≈10–30 ms, hook capture < 50 ms, client round-trip 2–5 ms, recall < 3.5 s hard limit.
- Build `neomemctl bench` early and record a baseline in benches/baseline.json.
- Once a feature is proven, profile it (samply or `neomemctl trace`), fix the biggest hotspot, and re-bench.
- Keep an optimization only if it is ≥3% faster on its bench with no regression elsewhere. Otherwise revert it.
- Each optimization gets its own `perf/` PR with before/after numbers.
- Release profile: lto = "fat", codegen-units = 1, panic = "abort".
- Any PR that makes a bench >5% slower must fix that before merging.

## Subagents: only for real jobs
Each subagent gets exactly one concrete job, with a clear deliverable and a command that proves it is done. Good jobs:
- "Build feature X in its own git worktree until `neomemctl prove X` passes, then open the PR."
- "Run benchmark Y and return the numbers."
- "Write fixtures for Z."
- "Read the docs for API W and return the exact request/response shapes."

Rules:
- Never more than 3 subagents at once, and only 2 if `free -g` shows under 16 GB total. Builders work on features with no shared files.
- **No review, audit, "double-check", "find issues" or "second opinion" subagents.** Proofs, tests, clippy, the xtask checks and benchmarks do the verifying.
- A reported problem counts only if it comes with a failing command and its output. Ignore any claim without one. Do not change working, proven code because someone thinks it could be better.
- Do not create a subagent for something you can do in a few tool calls yourself.

## Linux notes (the spec assumes macOS; support both)
- Daemon supervision: write a systemd user unit (`~/.config/systemd/user/neomemd.service`) for Linux and keep the launchd plist for macOS. `neomem install` picks the right one. Test the systemd one here.
- llama.cpp: detect the GPU. Use CUDA if `nvidia-smi` works, Vulkan if a Vulkan device exists, otherwise CPU with AVX2. Metal stays as a macOS cfg feature that is compiled but not run here.
- Socket path: same as the spec (~/.neomem/run/neomemd.sock). Use $XDG_RUNTIME_DIR only if the spec path is over the 108-byte limit.
- CI runs on ubuntu-latest and macos-latest.
- Speed budgets in §13.5 apply on this machine.

## Never ask, never stop on a blocker
- Never ask me questions. Decide using the spec, write it in docs/decisions.md, and keep going.
- If something truly needs me (a browser login for ChatGPT, for example):
  - build it against a fake in neomem-testkit so it is fully tested
  - mark the live check as `blocked: needs-user` in the feature graph
  - move on, and list it in the final report
- If a tool or download fails, try another way (another mirror, another crate, building from source) before marking anything blocked.

## Safety (hard rules)
- Never touch my real ~/.neomem data. Every run, test and bench uses a temporary isolated home: NEOMEM_HOME set to a temp dir, via `neomemctl up`.
- The only file you may read in ~/.neomem is ~/.neomem/secrets.env, to load JEV_API_KEY into the environment for live Jev tests. If it is missing, run Jev tests against the fake and mark live Jev checks `blocked: needs-user`.
- Never print, log, commit or put secrets in PR text. .gitignore covers secrets.env, *.env, models/ and target/. Run `git diff --cached` checks for key patterns before every commit.
- Never read or reuse pi, Hermes or Codex login tokens. neomem has its own ChatGPT login.
- A neomem data repo (the vault's git) never gets a remote. Only the source repo is pushed.
- Disk: run `cargo sweep --time 2` and remove finished worktrees after each merge. Keep the cargo target dir shared and under 20 GB.

## Done when
- Every feature in feature-graph.yaml is proven, apart from `blocked: needs-user` ones, and `neomemctl prove --all` is green on main.
- All §13.5 budgets are met in `neomemctl bench`.
- `neomem install` wires Claude Code, Codex and pi in an isolated home, and a scripted session captures and recalls through all three.
- A §16 baseline eval (LongMemEval-S plus the own coding set at minimum) is recorded in docs/eval.md.
- Then keep doing perf PRs until 3 rounds in a row each gain <3%.
- Finish with a short report in docs/STATUS.md: what was built, bench numbers vs budget, eval scores, and anything blocked on me.
