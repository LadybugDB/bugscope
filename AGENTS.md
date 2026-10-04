# AGENTS.md — bugscope-gpui contributor notes

## Before pushing: formatting, lints, tests

CI (`.github/workflows/build.yml`) enforces these — run them locally so
the build stays green:

```bash
cargo fmt --all -- --check   # must pass; run `cargo fmt --all` to fix
cargo clippy --all-targets -- -D warnings
cargo test
```

Do not push with `cargo fmt --all -- --check` failing.
