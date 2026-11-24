# Contributing to Rewind

Thanks for helping improve Rewind!

## Prerequisites

- Windows 11 (x64)
- Rust stable toolchain (`x86_64-pc-windows-msvc`)

## Building & Testing

```powershell
cargo check --all-targets
cargo test --lib
cargo build --release --bin rewind --bin rewind-daemon --bin Rewind-Setup
```

## Guidelines

1. Keep changes focused and minimal.
2. Ensure `RUSTFLAGS="-Dwarnings" cargo check --all-targets` passes with zero warnings.
3. Run `cargo test --lib` and include a unit test for non-trivial fixes or features.
4. Never add game-process memory reading/writing, DLL injection, DirectX hooks, or simulated input APIs.
5. Open a pull request against `main` using the PR template.
