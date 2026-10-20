# Decisions

Choices the spec leaves open, one line each, with the section they fill in.

- §13.8 `unicode-ident` is held at 1.0.24 in `Cargo.lock`, because `ra-ap-rustc_lexer` 0.178 refuses to build when it and `unicode-properties` disagree on the Unicode version.
- §17 CI runs on pushes to `main` only, and a newer push cancels the older run, so rapid merges do not queue macOS minutes.
- §17 The Actions workflow waits in `ci/workflows/ci.yml`: the gh token has no `workflow` scope, so `scripts/enable-ci.sh` (needs the user's browser sign-in) moves it into `.github/workflows/`. Until then `cargo xtask ci` runs the same steps locally before every merge.
- §13.2 `neomem-engine` may depend on `neomem-proto`, because `Engine::handle(Request) -> Response` takes the protocol's types.
- §13.2 Any crate may use `neomem-testkit` as a dev-dependency; runtime dependencies follow the §13.2 table, enforced by `cargo xtask check`.
- §17.4 Clippy pedantic runs without `missing_errors_doc`, `missing_panics_doc` and `must_use_candidate`: the first two demand doc comments, which the no-comments rule bans, and the third asks for an attribute on nearly every getter.
- §13.3 `Embedder::embed` takes a `Purpose` (query or document), because retrieval embedders such as EmbeddingGemma and Qwen3-Embedding use different prompts for each.
- §13.1 Spool files are removed once the daemon has handled them, and unreadable ones move to `spool/rejected/`; the spool is a transient queue, not memory. A crash between logging and removal can log a spooled add twice, which §2.4 already allows (no dedupe keys).
- §13.1 A write (`neomem add`) never starts the daemon: if neomemd is down it is spooled at once and drained on the next start; reads start the daemon and wait up to 2 s.
- §13.5 Proof scenarios run debug builds while other work shares the machine, so their timing claims are sanity bounds; the §13.5 budgets themselves are checked by `neomemctl bench` on release builds.
- §13.4 `neomem models test <role>` also takes `--source <name>` (checks a `[models.extractor_by_source]` override) and `--structured` (checks structured output), and reports when the first streamed text arrived.
- §13.4 `neomem login chatgpt` runs the sign-in in the daemon: the daemon binds the localhost callback, does PKCE, exchanges the code and writes `secrets.env`; the thin CLI only prints the address, opens the browser (`--no-browser` skips it) and waits up to 5 minutes, so it needs no HTTP client.
- §13.4 Every `Llm` adapter validates a structured answer against its schema (a JSON Schema subset: types, required, properties, closed objects, items, enum, anyOf) and retries once with the problem quoted; the validated JSON reaches `on_text` once, so a rejected answer never streams to the caller. Code fences around the JSON are dropped.
- §13.4 LLM roles without a `timeout` get 120 s.
- §13.4 Chat Completions sends `max_tokens` (what OpenAI-compatible servers accept) and effort as `reasoning_effort`, or `reasoning.effort` when the base URL is OpenRouter; OpenAI itself is reached with `api = "responses"`. A server that ignores `stream` and answers with one JSON body is read as such.
- §13.4 Anthropic effort goes to `output_config.effort` and thinking stays at the model's default (adaptive on current models); the system prompt carries `cache_control: ephemeral`; a request without a limit asks for 16,000 output tokens; thinking blocks never reach the text.
- §9.4 / §6.3 The nightly jobs run at `[schedule] nightly_at` (default 03:00, local time via `jiff`); `croner` is not needed for one fixed daily time.
- §17.2 `neomemctl set clock=…` works only on daemons started with `[dev] settable_clock = true` (written by neomemctl for isolated homes); the real daemon refuses it.
- §9.4 The drift check repairs by re-indexing the differing files, and rebuilds when an indexed memory has lost its file, so the index always follows the files.
