/goal Build neomem from ~/neomem/SPEC.md (the only source of truth) until done. Never ask me anything. Ubuntu 26.04 laptop, 8 cores, 30 GB, Intel Arc GPU, passwordless sudo, gh logged in. ~/neomem is a clone of private repo JohanSardar/neomem.

Setup: install anything needed with apt/cargo/npm/curl (cargo-nextest, cargo-deny, cargo-sweep, samply, hyperfine, node/pnpm, models). First PR: AGPL-3.0 LICENSE + CI (ubuntu + macos) running nextest, clippy pedantic -D warnings, cargo-deny, cargo xtask check, neomemctl prove --changed. Then §17 step 0 (xtask checks incl. no-comments lexer, neomemctl, .agents/skills/neomem-control/feature-graph.yaml), then follow `neomemctl map --next`.

Per feature (§17): pick via map --next; design the interface first if new (deep module, spec's seams); write the proof first and see it fail; make it pass (tests, clippy, `neomemctl prove <id>`); clean up; only prove changes status. Spec gaps: pick the simplest fit, log one line in docs/decisions.md.

Code: reads like prose. Comments banned; names explain. Small functions, typed IDs, no unwrap outside tests, no unsafe outside llama.cpp bindings, no delete method. Copy existing patterns, one way to do each thing. Fix repeated mistakes with types, then lints, then the skill.

Git: branch per slice (feat/<id>-<slice>, perf/, fix/, chore/). Small one-purpose commits (~20-150 lines), conventional messages, commit whenever it builds and tests pass. PR per working slice (1-5 commits); body = what, proof output, bench numbers if perf. Merge at once with `gh pr merge --merge --delete-branch`, pull main, continue. Many small real PRs; no empty or split-for-show PRs. Red CI on main is fixed first.

Speed: §13.5 budget (search 10-30 ms, capture <50 ms, client 2-5 ms, recall <3.5 s). Build `neomemctl bench` early, baseline in benches/baseline.json. After each proven feature: profile (samply / neomemctl trace), fix the top hotspot, re-bench; keep only >=3% wins with no regressions, each as a perf/ PR with before/after. Release: lto fat, codegen-units 1, panic abort. No PR may slow a bench >5%.

Linux: the spec assumes macOS; support both. systemd user unit on Linux, launchd plist on macOS, `neomem install` picks. llama.cpp: Vulkan here, CUDA if nvidia-smi, else CPU; Metal behind a macOS cfg.

Subagents: max 3 at once. Each gets one concrete job with a deliverable and a command proving it done: build feature X in its own worktree until prove passes then open the PR; run bench Y; write fixtures Z; read API W docs and return exact shapes. NO review/audit/double-check/find-issues/second-opinion agents; proofs, tests, lints and benches verify. A reported problem counts only with a failing command and its output; never change proven working code on opinion. Don't spawn agents for a few tool calls.

Blockers: never ask. If something needs me (e.g. ChatGPT browser login), build it against a fake in neomem-testkit, mark the live check `blocked: needs-user`, move on. If a download fails, try another route first.

Safety: never touch real ~/.neomem data; all runs use a temp NEOMEM_HOME via neomemctl up. Only read ~/.neomem/secrets.env to load JEV_API_KEY. Never print, log or commit secrets; gitignore *.env, models/, target/; check staged diffs for keys before commit. Never reuse pi/Hermes/Codex tokens. Vault data repos never get a remote. Git identity: JohanSardar + noreply email only. After merges: cargo sweep --time 2, remove worktrees, target dir <20 GB.

Done when: every feature proven (except blocked: needs-user) and prove --all green on main; all §13.5 budgets met; `neomem install` wires Claude Code, Codex and pi in a temp home and a scripted session captures + recalls through each; §16 baseline (LongMemEval-S + own coding set) in docs/eval.md; perf PRs until 3 rounds in a row gain <3%; docs/STATUS.md lists what was built, bench vs budget, eval scores, and anything blocked on me.
