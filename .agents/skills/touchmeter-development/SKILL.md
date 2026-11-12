---
name: touchmeter-development
description: Use when developing, reviewing, testing, or documenting Touchmeter Rust UI, CLI, simulator, preview, Linux hardware, session, data, or packaging behavior.
---

# Touchmeter development

Run all commands from the repository root. The root Cargo package owns the Rust
application and CLI; `ui/` is the Rune UI library. Useful source map:

- `src/main.rs`, `src/controls.rs`, and `ui/src/lib.rs`: app entry/runtime,
  controls, and Rune UI.
- `src/fixtures.rs`, `src/simdata.rs`, and `src/testcli.rs`: fixtures, simulated
  data, and CLI test-data injection.
- `src/live.rs`, `src/live_worker.rs`, `src/jobs.rs`, and `src/credentials.rs`:
  live provider data, background jobs, and credential access.
- `src/linux/data.rs`, `src/linux/runtime.rs`, and `src/linux/session.rs`:
  physical-panel data, Linux runtime, and session/safety policy.
- `tests/`: Rust integration and manual checks. `docs/` contains fixture,
  macOS validation, and Linux hardware recovery guidance; `protocols/` contains
  vendored protocol material. `pkgbuild/` and `bin/` support Linux packaging.

## Scope and validation

- Rust/Cargo is the application and test language. Prefer existing dependencies;
  add Rust dependencies only when justified by the task, and deliberately update
  the corresponding lockfile. Do not introduce Python, C++, Qt, or another
  application/test language. Non-Rust changes are only for minimal existing
  shell packaging/metadata when absolutely necessary and requested.
- Default features build the Qt-free fictional desktop simulator. The
  `linux-hardware` feature adds Linux physical-panel support. Physical support
  is Linux-only; native macOS simulator validation has not been performed.
- Required environment: Rust/Cargo, native graphics/window/font libraries,
  graphics loader and driver (software GPU such as Mesa lavapipe is suitable).
  Linux hardware builds also need the feature's Linux libraries. Initial builds
  may fetch dependencies. There is no pinned CI workflow; run checks manually.
- Offline tests must explicitly use `--simulator` whenever invoking the test
  CLI, to prevent touching physical test state. Do not run commands targeting
  physical hardware without explicit user context/approval.
- For preview/test commands that write state, isolate `XDG_RUNTIME_DIR` and
  `XDG_CONFIG_HOME` in user-owned temporary directories; runtime dir must have
  mode 0700. `mktemp` is sufficient; avoid adding helper scripts. Configuration
  is shared unless isolated. Example, from repository root:

  ```sh
  runtime=$(mktemp -d); chmod 700 "$runtime"
  config=$(mktemp -d)
  XDG_RUNTIME_DIR="$runtime" XDG_CONFIG_HOME="$config" cargo run -- preview "$runtime/preview.png" --fixture docs/fixtures/fire.json
  rm -rf "$runtime" "$config"
  ```

  This uses the explicit fictional fixture and temporary output/state only; do
  not modify real state. Never invoke actual credentials, input, audio,
  collectors/services, hardware, installation, or recovery commands without
  explicit user approval. Do not automatically launch the physical app/service;
  simulator and preview runs are appropriate when explicitly part of development.
- Typical checks (manual, from root):

  ```sh
  cargo fmt --check
  cargo fmt --check --manifest-path ui/Cargo.toml
  cargo test --locked
  cargo test --locked --manifest-path ui/Cargo.toml
  cargo test --locked --features linux-hardware # Linux only
  cargo build --release # Default features; compile only
  ```

## Safety invariants

- Unchanged fixture polling must preserve interactive application behavior,
  including picker, selection, and audio, for both physical data and simulator
  data. A changed fixture may intentionally apply its new initial scenario.
- All render paths, including dirty/incremental paths, use a monotonic clock.
- Keep centered layouts, at most two allowance groups, and stable hitboxes.
  First touch after wake only wakes; it must not activate another action.
- Preserve fresh-session authentication. Gates must not be weakened. Key release
  must stay synchronized; before acknowledging suspend or taking the safest
  shutdown path, the panel must be freshly confirmed OFF.
- `FaultHeld` retains resources; retain explicit documented recovery and never
  add automatic recovery/autorun.
- Preserve existing user work and all unrelated files. Preserve applicable
  third-party attributions and licenses when modifying assets or protocols.

## References

- Manual checks: [tests/MANUAL_CHECKS.md](../../../tests/MANUAL_CHECKS.md)
- macOS simulator validation: [docs/macos-validation.md](../../../docs/macos-validation.md)
- Linux hardware recovery: [docs/rust-hardware-recovery.md](../../../docs/rust-hardware-recovery.md)
