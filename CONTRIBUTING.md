# Contributing

Thanks for the interest. A few practical notes before you open a PR.

## Build and test

```
cargo build --release
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All four must pass. CI runs the same commands — but only matches your
results if you have [rustup](https://rustup.rs): the pin in
`rust-toolchain.toml` is what CI resolves, and without rustup your
package manager's Rust is used instead and the file is silently ignored.
A newer distro toolchain will fail on clippy lints CI never sees, and an
older one on syntax CI accepts.

## Scope

`podctl` aims at AirPods on Linux. Out of scope:

- non-AirPods devices (the AAP protocol is Apple-specific)
- iOS-only features (Find My, personalised spatial HRTF, hearing test)
- a TUI / GUI beyond the existing tray and popup

If in doubt, open an issue first.

## AAP protocol work

The most valuable contributions are confirmed AAP byte captures from
real devices — especially AirPods 4 ANC, AirPods Pro (gen 1) and
AirPods Max. Run `btmon` while toggling the setting on iOS, attach the
capture to the issue, and call out the setting that changed.

Do not paste GPL-licensed code into this repo. The crate is licensed
MIT OR Apache-2.0 and must stay compatible.

## Style

- No decorative comment separators, no module-header essays.
  Code that needs an explanation gets a one-line `//` comment.
- Errors follow `podctl: <verb>: <msg>` for the CLI, `anyhow::bail!`
  inside library code.
- `#[allow(dead_code)]` needs a comment above it justifying why.
- New `unsafe` blocks need a one-line `// SAFETY:` rationale.

## Commit messages

Short imperative subject, optional body explaining *why* (the diff
already shows *what*). One logical change per commit.

## Signing off

By submitting a PR you agree your contribution is licensed under
MIT OR Apache-2.0, same as the rest of the project.
