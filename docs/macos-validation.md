# Manual macOS validation

This checklist is for the Rust desktop simulator, preview renderer and CLI.
Physical Apple Touch Bar access is Linux-only and is not supported on macOS.
Do not run a bare physical-target `test` command: root commands default to the
physical test-state path. All state-changing examples below explicitly use
`--simulator`. This checkout has not been manually validated on macOS.

## Setup and build

Use a copy of the current checkout (the repository is not published; do not use
a fabricated `git clone` URL). Build natively for the host architecture, Apple
Silicon or Intel; this does not produce a universal binary.

```sh
xcode-select --install
```

Install stable Rust with rustup, following https://rustup.rs/ (including its
instructions to configure the shell). Record macOS version, machine architecture,
Rust version, and GPU.

```sh
rustc --version
cargo --version
cargo test --locked
cargo build --release --locked
```

The Rust default build requires native OS graphics, window and font libraries.
Optional dependency inspection:

```sh
cargo tree --locked -e features
```

The default dependency graph should not require DRM, evdev, zbus or SQLite.
Report discrepancies rather than changing dependencies as a workaround.

## Isolated simulator checks

Give the CLI private XDG runtime/config/state/data locations so validation cannot
touch real test or preference files. Keep `HOME` intact if the system needs its
installed fonts. The simulator uses fictional data and does not probe credentials.

```sh
testroot="$(mktemp -d)"
chmod 700 "$testroot"
mkdir -m 700 "$testroot/runtime" "$testroot/config" "$testroot/state" "$testroot/data"
export XDG_RUNTIME_DIR="$testroot/runtime"
export XDG_CONFIG_HOME="$testroot/config"
export XDG_STATE_HOME="$testroot/state"
export XDG_DATA_HOME="$testroot/data"
```

Launch `cargo run --release -- simulate --scenario fire` and check menu, F, AI,
provider picker, volume and back controls. Verify signed-out, stale, unavailable
provider and missing-limit states; sliders/mute remain simulated. Confirm F-key
highlighting without desktop key injection, the TEST badge, and that no audio
changes or authentication data are accessed.

Check all-page PNG output and dimensions (2008×60):

```sh
for page in menu ai functions providers volume; do
  cargo run --release -- preview "$testroot/$page.png" --demo --"$page"
  sips -g pixelWidth -g pixelHeight "$testroot/$page.png"
done
cargo run --release -- preview "$testroot/da.png" --demo --ai --language da
cargo run --release -- preview "$testroot/fire.png" --scenario fire
```

Also verify `--demo`, Danish language, invalid-language/error handling, scenario
selection and fixture preview. For fixture watching, edit a valid fixture and
confirm atomic reload, then test malformed and deleted files fall back to the
fictional menu. Generated scenario data has a frozen clock while fire animation
continues; loaded fixtures retain their supplied clock behavior.

In the window, test monitor listing and selection by index and exact name,
fullscreen on a selected monitor, F11/Escape/M, resizing, Retina scaling and
coordinate accuracy, and multiple-monitor placement. Hold Fn/key input and release
after focus loss; drag the volume slider across other controls and confirm pointer
capture. Confirm close/focus loss releases held interactions. No key should be
injected and no audio device should be controlled.

With the simulator open, exercise simulator-only CLI state:

```sh
target/release/touchmeter test set claude --enabled --short 95 --burn-minutes 20 --simulator
target/release/touchmeter test clear --simulator
target/release/touchmeter config set fire.trigger-minutes 10
target/release/touchmeter config set fire.trigger-minutes 60
```

Check updates appear, and clearing restores the launch scene. Config changes
should update fire behavior; restore the default in the isolated config with
`target/release/touchmeter config reset fire.trigger-minutes`. Confirm Ctrl-C and
SIGTERM both exit cleanly with status 0.

## Results to report

For each item mark PASS, FAIL or NOT RUN; include reproduction steps and relevant
logs for failures. Do not include credentials, tokens, or personal auth data.

| Check | Result / notes |
| --- | --- |
| macOS version / machine / OS architecture | |
| Rust version / GPU | |
| `cargo test --locked` | |
| Release build | |
| Simulator interactions / isolation | |
| Preview pages / PNG dimensions | |
| Fixtures / clock behavior | |
| Monitor / Retina / fullscreen | |
| CLI / configuration / signals | |
| Errors and log locations (redacted) | |
