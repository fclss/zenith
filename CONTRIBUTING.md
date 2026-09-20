# Contributing to zenith

Thanks for taking the time to contribute. This document covers the essentials;
keep changes focused and small.

## Development setup

- Linux with `rust` (stable, 1.75+) and `cargo`.
- [sing-box](https://sing-box.sagernet.org/) on `PATH` for the real core. Most
  development and all unit tests run against the in-process fake core, so
  sing-box is only needed for end-to-end work.
- For TUN-mode testing: passwordless `sudo` (`sudo -n`) and `ip`/`resolvectl`.

```sh
git clone https://github.com/fclss/zenith
cd zenith
cargo build
cargo run -- --fake   # exercise the UI without spawning a core
```

## Before you open a PR

All of these must pass locally — CI runs the same:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

## Guidelines

- **Keep the core seam intact.** Everything outside the TUI goes through the
  `Core` trait (`src/core/mod.rs`). Add behaviour there — with a fake
  implementation — rather than reaching for `Command::new` from the UI.
- **No comments unless they explain *why*.** The code should read on its own;
  reserve comments for non-obvious decisions.
- **Tests come with the change.** Prefer exercising the fake core and the UI
  renderer over live sing-box.
- **Record architecture decisions.** If you make a decision a future reader
  would question, add a short ADR under `docs/adr/` following the existing
  format.
- **Conventional commits.** `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`, etc.

## Reporting bugs

Open an issue with: what you did, what you expected, what happened, and the
output of `zenith --proxy-env` plus your `settings.yaml` with secrets redacted.
For tunnel issues, include the tail of `~/.config/zenith/sing-box.err`.

## License

By contributing you agree that your contributions are licensed under the
MIT License (see [LICENSE](LICENSE)).
