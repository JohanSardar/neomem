# Working on neomem

SPEC.md is the only source of truth. The build method is §17; the control skill is `.agents/skills/neomem-control/SKILL.md`.

- Pick work with `neomemctl map --next`; a feature is done only when `neomemctl prove <id>` passes.
- Gate every merge with `cargo xtask ci` (format, build, nextest, clippy pedantic, cargo-deny, xtask check, prove --changed).
- No comments in Rust. No `unwrap` outside tests. No `unsafe` outside the llama.cpp bindings. No delete methods.
- Copy the existing pattern for any recurring job; never add a second way to do the same thing.
- When a mistake repeats, fix it at the lowest rung that works: types, then a lint or xtask check, then this file.
- Never touch the real `~/.neomem`; every run uses an isolated home through `neomemctl`.
- Spec gaps get one line in `docs/decisions.md`.
