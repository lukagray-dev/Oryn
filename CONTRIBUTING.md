# Contributing to Oryn

## Setup

```bash
git clone https://github.com/lukagray-dev/oryn.git
cd oryn
cargo build
```

Requires Rust (stable) and the Slint toolchain dependencies for your platform,
see [Docs](https://github.com/slint-ui/slint/blob/master/docs/building.md).

## Workflow

1. Open an issue before starting large changes.
2. Fork, branch, commit with clear messages.
3. Run `cargo fmt` and `cargo clippy` before submitting.
4. Open a PR against `main`.

## Code style

- Standard `rustfmt` defaults.
- Prefer small, focused crates/modules over monoliths.
- No unwrap() in library code paths reachable from user input; use `Result` +
  `thiserror`.

## Reporting bugs

Use the issue tracker. Include repro steps, expected vs actual behavior, and
platform/OS.
