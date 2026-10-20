# Decisions

Choices the spec leaves open, one line each, with the section they fill in.

- §13.8 `unicode-ident` is held at 1.0.24 in `Cargo.lock`, because `ra-ap-rustc_lexer` 0.178 refuses to build when it and `unicode-properties` disagree on the Unicode version.
- §17 CI runs on pushes to `main` only, and a newer push cancels the older run, so rapid merges do not queue macOS minutes.
- §17 The Actions workflow waits in `ci/workflows/ci.yml`: the gh token has no `workflow` scope, so `scripts/enable-ci.sh` (needs the user's browser sign-in) moves it into `.github/workflows/`. Until then `cargo xtask ci` runs the same steps locally before every merge.
- §13.2 `neomem-engine` may depend on `neomem-proto`, because `Engine::handle(Request) -> Response` takes the protocol's types.
- §13.2 Any crate may use `neomem-testkit` as a dev-dependency; runtime dependencies follow the §13.2 table, enforced by `cargo xtask check`.
