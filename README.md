![Touch Bar app launcher](docs/menu.png)

# Touchmeter

Touch Bar apps for the Apple silicon MacBook, rendered with
[Rune](https://github.com/anomalyco/rune). Starts at an extensible launcher with
**F**, **AI**, and a **speaker icon** for Volume. Each app has a **back arrow at
the far right** that returns to the launcher; navigation activates on release
inside its original button. A shared app catalog in `src/controls.cpp` defines
launcher entries and hit targets, ready for more apps without another mode toggle.

## Rust desktop simulator (default)

The repository-root Cargo executable is the default build and interactive desktop
simulator. It uses winit 0.30 and softbuffer with Rune's typed UI API; it does not
require Qt. The simulator, preview PNG renderer, CLI, configuration, language and
monitor selection are ported. Native macOS support is intended but has not been
validated; CI configuration is not evidence of a successful macOS run. See
[`docs/macos-validation.md`](docs/macos-validation.md) for the manual checklist.

The hardware feature modules remain experimental and partial. A safety audit and
real-hardware validation are still blocked; root physical-panel access is rejected.
C++/Qt remains the physical backend until its replacement is verified. Do not treat
the experimental `PKGBUILD.rust` as a supported installation workflow.

```sh
cargo run --release -- simulate
cargo run --release -- simulate --scenario fire
# Equivalent Makefile wrappers:
make rust-simulator
make run-rust-simulator ARGS="--scenario fire"
cargo test --locked
```

The Rust simulator supports all ten built-in scenarios, page flags (`--menu`, `--ai`,
`--functions`, `--providers`, `--volume`), `--fixture`, `--monitor` (index or name),
and `--fullscreen`. F11 toggles fullscreen; Escape leaves fullscreen or closes the
window; M opens the menu. Mouse buttons, provider picker, F-key highlighting, and
simulated volume mute/drag are interactive. Live fixture polling safely falls back
to the fictional menu if a fixture is invalid or deleted. Data remains fictional:
Rust simulation never accesses real audio, injects desktop keys, or runs collectors.
Monitor listing is supported. Build natively for your platform; the executable still
needs OS graphics/window and font libraries.

```sh
cargo run --release -- simulate --fixture docs/fixtures/fire.json
```

The Rust root `test` CLI supports the existing command set, scenarios, `load`,
`status`, and `clear`; `--simulator` may appear anywhere after `test` to target
simulator windows. Simulator overrides win over the base fixture or scene, poll
every 500 ms, and clearing them restores the latest valid base. Generated simulator
data uses a frozen clock; loaded fixtures retain their supplied clock. Without
`--simulator`, the CLI targets the physical test-state path, which can affect an
existing legacy physical service through the shared state file; root commands do
not all operate in memory. Rust simulator state itself has no physical side effects.

```sh
target/release/touchmeter test set claude --enabled --short 95 --burn-minutes 20 --simulator
target/release/touchmeter test clear --simulator
```

The Rust `config` CLI supports `path`, `show`, `get`, `set`, and `reset`, including
the live fire-threshold setting, polled every 500 ms. CLI state files are compatible
with C++ targets; no existing installed service behavior is changed by this
experimental migration.

```sh
target/release/touchmeter config path
target/release/touchmeter config show
target/release/touchmeter config get fire.trigger-minutes
target/release/touchmeter config set fire.trigger-minutes 60
target/release/touchmeter config reset fire.trigger-minutes
```

Legacy C++ build paths remain available, and the installed service is unchanged.
Rust hardware modules/adapters are not yet a validated physical backend; real audio,
credential access, collectors and desktop key injection must not be assumed ported.

The sections below describe shared and legacy workflows; commands that explicitly
use `build-legacy` or `run-legacy-simulator` select the C++/Qt implementation.

## Desktop simulator

### Legacy C++/Qt standalone build (Linux and macOS)

No Omarchy installation or Touch Bar is needed. This build excludes DRM leasing,
evdev, virtual keyboard injection, session D-Bus, and generated Wayland protocols.
It uses bundled provider logos and the default palette. Audio and F keys are
always fictional. A GPU backend is still needed: Metal on macOS, Vulkan on Linux
(Mesa software Vulkan can be used for headless tests).

Build prerequisites: a C/C++ compiler, Make, Qt 6 development tools, and a current
stable Rust/Cargo toolchain (`rustup`). Build natively on each operating system
and CPU architecture; a Linux binary will not run on macOS.

```sh
# macOS: install Xcode Command Line Tools, Homebrew, and Rust via rustup first.
xcode-select --install
brew install qt
# Linux (Debian/Ubuntu), in addition to Rust:
sudo apt install build-essential pkg-config qt6-base-dev qt6-base-dev-tools qmake6 \
  libfontconfig1-dev libvulkan-dev mesa-vulkan-drivers libxkbcommon-dev libwayland-dev
# On a Wayland desktop, also install qt6-wayland.

make build-legacy-simulator
make run-legacy-simulator
make run-legacy-simulator ARGS="--scenario fire --fullscreen"
make test-legacy-simulator
```

If Qt isn't discoverable, set `QMAKE=/path/to/Qt6/bin/qmake`. Homebrew Qt is
detected automatically. The executable is `build/simulator/touchmeter-simulator`;
it opens the simulator with no arguments. You can also use `bin/simulate` to
build and run it. It links your installed Qt libraries; it is not a self-contained
macOS app bundle or distributable installer.

```sh
build/simulator/touchmeter-simulator --scenario fire
build/simulator/touchmeter-simulator test set claude --enabled --short 95
build/simulator/touchmeter-simulator test clear
```

The standalone binary's `test` commands **always target simulator state**
(`--simulator` is accepted but optional). With no `XDG_RUNTIME_DIR`, it creates
a private per-user runtime directory under the OS temporary directory, so the
CLI and running windows share overrides on macOS too. `preview` and `config`
remain available; `open` is explicitly rejected.

### Full installation

Demo the same Touch Bar on a normal monitor, without leasing the physical panel:

```sh
touchmeter simulate
touchmeter simulate --scenario fire
touchmeter simulate --volume
touchmeter simulate --scenario volume-muted
touchmeter simulate --fixture ~/Work/touchmeter/docs/fixtures/fire.json

# List connected monitors, then present fullscreen on one (index or exact name).
touchmeter simulate --list-monitors
touchmeter simulate --monitor 1 --fullscreen --scenario fire
```

This is a **movable, resizable desktop window running the real Rune scene**, not
a video or a separate UI mock-up. Click the launcher/back/provider buttons, press
the F keys with the mouse, and **drag the volume slider**. F-key presses highlight
the actual buttons and appear in the window title, but never inject desktop keys.
Volume and mute work locally in the simulator and never read/change real audio.

**F11** toggles fullscreen, **Escape** leaves fullscreen (or closes a normal
window), and **M** returns to the menu. The bar keeps its physical aspect ratio
and is letterboxed when necessary; letterbox/footer clicks do nothing. Resize,
focus loss and close cancel held contacts/keys. Fire animates while shown and
pauses when the window is unexposed/minimized.

The simulator defaults to fictional all-provider data at the menu, with a **TEST**
badge. It does not read credentials or collect usage. Built-in scenarios use a
frozen data clock so demo limits don't expire mid-presentation; flame animation
still moves. The same `--menu`, `--ai`, `--functions`, `--providers`, and `--volume`
page flags used by previews can override the initial page. Scenarios are the same
ones listed by `touchmeter test --help`. `--demo` uses the standard yellow palette;
otherwise the simulator follows your desktop theme. `--language` is supported.

Explicit fixture files reload live **inside that simulator only**. A bad/deleted
fixture falls back to the fictional menu, never live accounts/audio. The simulator
ignores the physical bar's `$XDG_RUNTIME_DIR/touchmeter-test.json` and can
run alongside the hardware service or other simulator windows. Configuration is
still shared/read live, including the fire trigger threshold. Closing a simulator
does not stop the physical Touch Bar.

Use the **same artificial-data commands for either target**; add `--simulator`
anywhere after `test` to affect simulator windows instead of the physical bar:

```sh
# Physical Touch Bar (TEST mode, not real provider quotas)
touchmeter test set claude --enabled --short 95 --weekly 61 --burn-minutes 20

# Simulator windows only
touchmeter test set claude --enabled --short 95 --weekly 61 --burn-minutes 20 --simulator
touchmeter test scenario fire --simulator
touchmeter test status --simulator
touchmeter test load fixture.json --simulator
touchmeter test clear --simulator
```

`--short` and `--weekly` are fictional **percentages used**; `--burn-minutes` is
a fictional exhaustion forecast. These never change provider-enforced quotas.
Simulator overrides live in `$XDG_RUNTIME_DIR/touchmeter-simulator-test.json`,
apply to all simulator windows within 0.5 seconds, and take precedence over launch
scenarios/fixtures/page flags. Clearing them restores each window's launch scene
(or its latest valid watched fixture). Invalid/deleted launch fixtures fall back
to the fictional menu. Generated simulator data freezes its clock; loaded fixtures
keep their supplied clock behavior. Clear without `--simulator` restores physical
live usage only. Neither target changes the other's state. Persistent `config`
settings, such as the flame threshold, remain shared.

On Wayland, the compositor controls normal window placement: move it to any monitor
using your usual window-manager drag/move controls. `--monitor` selects the requested
screen where supported; combining it with `--fullscreen` requests that monitor
explicitly. No Hyprland monitor/window rules are edited. A desktop display connection
is required; `simulate --help` works without one.

## AI usage

The AI app shows the **percentage used**
and reset countdowns for the first two allowance windows reported by Omarchy.
These are provider/CLI allowances, not ChatGPT chat-message limits.

![Rune Touch Bar displaying example usage](docs/preview.png)

Tap the **provider logo** at the left to open/close the inline provider picker.
Tap another enabled logo to switch its usage onto the bar. Providers without
credentials are **gray and cannot be selected**, even if old usage is cached.
Enabled logos retain their standard brand colors (for example, Claude orange),
independent of the desktop accent.
Credential availability is checked every fifteen seconds; tokens never enter the UI.
Supported providers: OpenAI/Codex, Anthropic/Claude, xAI/Grok, Kimi, DeepSeek,
MiniMax, Z.ai, OpenRouter, Synthetic, Ollama Cloud, OpenCode Zen, and Cursor.
Credentials do not guarantee a subscription usage endpoint: API-key-only accounts
may show unavailable limits rather than a fabricated allowance.

## Function keys

Open **F** from the menu for **F1–F12**. The back arrow returns to the menu,
not directly to AI. Buttons have an 8-pixel corner radius, independent of desktop window
rounding. Function keys have real press/release behavior and work with
the physical keyboard's modifiers through Hyprland's shared keyboard state.
Only one Touch Bar finger is handled at a time; sliding off releases a key
without typing the adjacent key. Leaving an app releases any held key.

![Function keys with the menu back button](docs/functions.png)

![Inline provider picker; unavailable providers are gray](docs/providers.png)

## Volume

Open the **speaker icon** in the launcher. Touch anywhere on the slider and
**slide your finger** to set the volume continuously; no plus/minus buttons are
needed. The slider captures the contact, so sliding across the mute/back buttons
cannot accidentally activate them. Release ends the drag.

![Draggable output-volume slider, mute button and menu back button](docs/volume.png)

Tap the speaker at the far left to **mute/unmute**. A crossed-out red speaker
indicates mute. Adjusting the slider while muted changes the stored level but
does **not** unexpectedly unmute. Volume is capped at **100%** (no amplification).
The percentage and knob follow changes made with the keyboard or other apps.

The backend uses unprivileged `wpctl` calls against `@DEFAULT_AUDIO_SINK@`, so it
follows the system's current output device. Queries run about once a second only
while the volume app is visible, awake and unlocked. Writes are serialized and
coalesced at up to 25 updates/second while dragging; the latest position always
wins. No shell commands, media-key injection or administrator access are involved.
Missing/disconnected audio disables the slider and mute button, displays an error,
and retries; the menu back button remains available. Lock/sleep/navigation cancel
queued writes, and a wake-only touch never adjusts volume or toggles mute.

For safe testing, **every test fixture** makes Volume simulated, even a fixture
originally intended for AI. Simulated sliders and mute never invoke `wpctl`.
Use `touchmeter test clear` before expecting real audio control.

## Burning through a limit

The AI **usage screen catches fire** when the observed consumption rate projects
any quota running out within **30 minutes by default**, before that quota resets.
The threshold is configurable with `touchmeter config set fire.trigger-minutes <minutes>`. Rune's
`rune-fx` GPU fire shader animates flames and embers behind the metrics at about
30 fps; the flames intensify as the estimate falls. Brand-colored logos, stable
touch targets, a dark readability scrim and an approximate time-to-limit stay visible.

![Animated quota fire, shown here as one frame](docs/fire.png)

This is an estimate from **recent allowance growth**, not a token counter or a
percentage-used threshold. A 95%-used quota at an idle/slow pace does not trigger it.
Touchmeter keeps up to fifteen minutes of in-memory samples for the selected
provider, requiring two fresh collector measurements at least a minute apart.
With the normal five-minute collector interval, it needs at least the next refresh
after launch/provider selection. Account changes, window resets, falling usage,
stale records and missing credentials discard the affected baseline. Unknown reset
times cannot produce a forecast. Quiet periods cool the estimated rate. Histories
are not saved across app restarts and test data never trains the real estimator.

Fire is confined to AI usage: hidden in the menu, Volume, provider picker and F keys. Its animation timer stops
when hidden, idle, locked, inactive or unleased; existing OLED/suspend protections
remain in force. It does not wake the panel or prevent sleep. No simulation runs
in the background when the screen is not burning.

Try it without consuming tokens (after installing this version):

```sh
touchmeter test scenario fire
touchmeter test set claude --burn-minutes 5
touchmeter test set claude --no-burn
touchmeter test clear
```

`fire` supplies a fictional 20-minute estimate. `--burn-minutes` accepts 0–1440;
values above the configured trigger do not light the fire. Disabled, stale or missing-usage fixtures
still suppress it. A frozen-clock example is in `docs/fixtures/fire.json`.

## How it works

- Leases only `DSI-1` through Hyprland's standard Wayland DRM-lease protocol.
  Refuses to configure any panel that is not exactly 60×2008 pixels. Your laptop
  and external monitor remain ordinary, independent desktops; no monitor rules
  are changed. A Touchmeter-only entry in `~/.config/hypr/input.lua` enables
  shared modifiers and releases held F keys if the app crashes or stops:

  ```lua
  hl.device({ name = "hl-virtual-keyboard-touchmeter", share_states = 1, release_pressed_on_close = true })
  ```

  This does not alter the physical keyboard or trackpad.
- Renders a 2008×60 Rune scene into an offscreen GPU texture, reads it back,
  and rotates it into the leased DRM dumb buffer. Updates on input/data changes
  and every fifteen seconds. The Rune revision and Cargo dependencies are pinned.
  A small Rust static library owns the UI; the existing C++/Qt Core/Gui/DBus host
  retains display leasing, session policy, raw touch and virtual keyboard safety.
  There is no Qt Quick/QML rendering or desktop window.
- Reads `~/.local/state/omarchy/agents/usage/<provider>.json`. Every five minutes
  and when switching providers it runs
  `omarchy agent usage update --limits-only <provider>` through Omarchy's existing
  collector. A separate read-only Python helper uses the installed collectors'
  credential readers and returns only availability booleans. It never calls their
  network/refresh routines, logs credentials, or writes authentication files.
  The UI receives no keys/tokens. Stop the service to stop refresh requests.
- Follows Omarchy's background, foreground, accent, warning colors, corner
  rounding, and language live. English and Danish translations
  are included; other languages fall back to English. Data is always plain text.
- Powers the panel off during suspend, lock, logout, an inactive desktop,
  or when all desktop monitors are powered off.
  A logind delay inhibitor allows it to shut down the panel before
  suspend proceeds; it does not prevent sleep. Hyprland's lock state is checked
  every second as well as logind's session state. Touch cannot wake the panel or
  send function keys or change audio while the session is locked/inactive. It stays off if session
  checks fail, and resumes only in an active, unlocked Wayland session.
- Shifts the scene slightly each minute and powers off the OLED after two minutes
  without input, including when a video player inhibits desktop idling (with
  idle-notify protocol version 2). Keyboard/mouse activity restores the display.
  Touching a blank Touch Bar also wakes it; that first touch does not type a key
  or switch modes. Only the usage text and key/button labels shift, so hit targets
  do not move.
  These measures reduce, but cannot eliminate, OLED burn-in risk.
- Uses the compositor's lease and virtual keyboard, without running as root or
  granting broad `/dev/dri`, keyboard, trackpad, or `/dev/uinput` permissions.
  A packaged udev rule grants only the active local user access to the Apple
  silicon Touch Bar's `apple_z2` input device. Touchmeter exclusively grabs that
  device while running, so taps cannot also click things on your main screen.
  If a lease is revoked on suspend/VT switching, the user service retries.
- Missing/invalid limits show an error instead of zero usage. Old data or
  failed refreshes are marked stale. Unknown reset times are explicitly labeled.
  Taps on the usage metrics themselves do nothing. Selection starts with Codex
  on each launch; provider switching here does not change other AI applications.

## Build and install

The default `make` / `bin/build` path builds the Qt-free Rust executable; it needs
a stable Rust toolchain plus platform graphics, windowing and font libraries.
Use the explicit `build-legacy` / `bin/build-legacy` paths for the C++/Qt backend.
The installed service and existing installation workflow remain legacy and unchanged.
Do not infer that the experimental Rust package is installable or hardware-ready.

Legacy dependencies: Rust/Cargo (edition 2024), Qt 6 base, libdrm, libevdev, libxkbcommon,
wayland-client, wayland-protocols, qmake6, make, a C++17 compiler, Python 3, Omarchy,
brightnessctl, Qt 6 Wayland (for the desktop simulator), WirePlumber (`wpctl`), a PipeWire audio session, Vulkan loader and a
working Vulkan driver (or Mesa lavapipe).
The first build downloads Rune and Cargo dependencies.

```sh
make
cargo test --locked
bin/build-legacy
bin/test-legacy
bin/install
```

The installer builds a local Arch package, asks for administrator authorization
(`sudo` in a terminal, a graphical prompt otherwise), installs/reloads the narrowly
scoped udev rule, and enables/restarts `touchmeter.service` for your user.
Source stays in `~/Work/touchmeter`. No repository is published or added.

The service sets Touch Bar brightness to 25% at startup. To adjust it now:

```sh
brightnessctl -d 228600000.dsi.0 set 40%
```

## Control

```sh
systemctl --user status touchmeter.service
systemctl --user restart touchmeter.service
systemctl --user disable --now touchmeter.service
journalctl --user -u touchmeter.service -n 30
```

Disabling stops startup and releases the display, touch grab, and virtual keyboard.
The package remains installed and can be removed separately with the usual package manager.

The desktop launcher entry starts the Touch Bar app menu. A runtime lock prevents
multiple instances from competing for its lease.

## Configuration

Preferences are persistent and apply to the running bar within about half a second,
without a restart or administrator access. They apply to real usage, test mode and
offscreen previews alike.

```sh
touchmeter config show
touchmeter config get fire.trigger-minutes

# Start flames when the recent pace predicts exhaustion within an hour.
touchmeter config set fire.trigger-minutes 60

# Wait until only fifteen projected minutes remain instead.
touchmeter config set fire.trigger-minutes 15

# Restore this setting (or all settings) to defaults.
touchmeter config reset fire.trigger-minutes
touchmeter config reset
touchmeter config path
```

| Setting | Default | Allowed values | Meaning |
| --- | --- | --- | --- |
| `fire.trigger-minutes` | `30` | Integers `1`–`1440` | Flames when projected remaining quota time is at or below this many minutes. Larger values warn earlier. |

This changes the **time-to-exhaustion trigger**, not the token count or percentage
used, and scales flame intensity relative to that threshold. It does not bypass
stale-data, credential, quota-reset or session-safety checks.

The file is `$XDG_CONFIG_HOME/touchmeter/config.json`, falling back to
`~/.config/touchmeter/config.json`:

```json
{
  "version": 1,
  "fire": { "triggerMinutes": 30 }
}
```

Reading defaults does not create files. CLI changes validate known keys and ranges,
serialize concurrent writers with a lock, save atomically with owner-only file
permissions, and back up an existing file as `config.json.bak.<UTC timestamp>.<pid>`.
Symlinked config files cannot be overwritten by the CLI. Invalid/future-version
files are never silently replaced, even by `reset`: back up/fix the file first.
An invalid manual edit leaves a running app's last valid settings in effect and logs
a warning; invalid startup config falls back to defaults. Removing the file restores
defaults. `touchmeter config --help` lists commands and settings.

Configuration is separate from `$XDG_RUNTIME_DIR/touchmeter-test.json`.
`touchmeter test clear` leaves preferences unchanged; `config reset` does not clear
test data. To try a threshold with the fire fixture's fictional 20-minute forecast,
set the trigger to `15` (no flames) and then `30` (flames).

## Preview and tests

### Inject test data with the CLI

```sh
# Enable every logo and open the picker, even without credentials.
touchmeter test scenario all-providers

# Select Claude and simulate its usage (CLI percentages are 0–100).
touchmeter test set claude --enabled --short 96 --weekly 61
touchmeter test set claude --stale
touchmeter test set claude --disabled --mode providers

# Other presets: high-usage, stale, missing, signed-out, fire, menu.
touchmeter test scenario missing

# Test audio controls without changing real system audio.
touchmeter test scenario volume
touchmeter test scenario volume-muted
touchmeter test scenario volume-unavailable

# Export, edit, and reload any supported fixture.
touchmeter test status > /tmp/opencode/touchmeter-fixture.json
touchmeter test load /tmp/opencode/touchmeter-fixture.json
touchmeter test load ~/Work/touchmeter/docs/fixtures/claude-high.json

# Return to real credentials/usage. Safe to run more than once.
touchmeter test clear
```

The running app picks up changes within about half a second, without restarting.
**TEST** is displayed in every mode; usage status also has a TEST prefix.
The CLI itself needs no GPU, display connection, administrator access, or credentials.
State lives only in `$XDG_RUNTIME_DIR/touchmeter-test.json` (owner-only permissions),
persists across app restarts, and is normally removed when the user runtime directory
is cleared at logout. Use `clear` when done. Invalid fixtures are rejected without
replacing the current state; an externally corrupted/deleted runtime fixture returns
the running app to live mode. CLI success means the fixture was saved, not that the
service is running or the physical panel is awake.

While test mode is active, credential probes and automatic usage refreshes are paused.
The app stops its existing refresh process; collector children already launched by
Omarchy may still finish. Real credentials, real usage records and account
settings are never overwritten by injection. `clear` resumes normal collection.
Lock, suspend, inactive-session, idle/OLED and function-key safety rules are **not
overridable** through fixtures. This is data injection, not synthetic keyboard input.

Fixture JSON uses `version: 1`, `selected` (provider ID), `mode` (`usage`, `providers`,
`functions`, `menu`, or `volume`), and `providers` keyed by the supported provider IDs. Each entry has
boolean `enabled` and `ready` fields and a `limits` array in the Omarchy format:
`label`, `percent` (**0–1 fraction**), and ISO `resetsAt` (or `""` for unknown).
Optional fields: `limitsStale`, millisecond `limitsFetchedAt`, `usageStatusText`.
The test-only `burnMinutes` field simulates the projected minutes to exhaustion;
it is never accepted as a real collector forecast.
An optional top-level `volume` object has `percent` (0–100), `muted` (boolean),
and `available` (boolean); defaults are 65%, unmuted and available. A volume-only
example is in `docs/fixtures/volume.json`. Omitted volume data is still simulated
in test mode. `clear` returns to the menu and resumes live data/audio access.
An optional top-level ISO `now` freezes countdown/staleness calculations for repeatable
previews. Providers omitted from a fixture are disabled; no live data is mixed in.
An empty `limits` array simulates unavailable limits; past resets simulate reset-due.
`set` selects the named provider and defaults to usage mode, retaining its enabled
state unless `--enabled`/`--disabled` is given. Usage changes retain stale state unless
`--fresh` is given. `touchmeter test --help` lists all commands.

### Offscreen previews

```sh
touchmeter preview /tmp/opencode/touchmeter.png --demo
touchmeter preview /tmp/opencode/touchmeter-ai.png --demo --ai
touchmeter preview /tmp/opencode/touchmeter-da.png --demo --ai --language da
touchmeter preview /tmp/opencode/touchmeter-fkeys.png --demo --functions
touchmeter preview /tmp/opencode/touchmeter-providers.png --demo --providers
touchmeter preview /tmp/opencode/touchmeter-volume.png --demo --volume
touchmeter preview /tmp/opencode/touchmeter-fixture.png --fixture ~/Work/touchmeter/docs/fixtures/claude-high.json
touchmeter preview /tmp/opencode/touchmeter-fire.png --fixture ~/Work/touchmeter/docs/fixtures/fire.json
touchmeter preview /tmp/opencode/touchmeter-fire.png --scenario fire
```

Preview runs the real Rune scene offscreen and saves a PNG. It does not lease
the Touch Bar or make refresh requests. Demo data is fictional and uses the
default yellow accent. `--language` changes only that preview, not your desktop.
`--fixture` renders a supplied test fixture without changing the running bar or
reading credentials. It cannot be combined with `--demo`. Normal previews ignore
live injection unless a fixture is explicitly supplied.
`--scenario <name>` selects a built-in fictional scenario without writing live test
state; it cannot be combined with `--fixture`. Interactive demos use `simulate`.
With no page flag or fixture, previews show the menu. Choose at most one of
`--menu`, `--ai`, `--functions`, `--providers`, or `--volume`. All previews use
simulated volume and never read/change real audio.

The background renderer intentionally does not capture desktop keyboard input.

Backend tests cover valid allowances, weekly/reset formatting, malformed and
oversized records, missing values, out-of-range percentages, expired windows,
and markup-shaped labels. Controller tests exercise all twelve function keys,
menu/back navigation, drag-off releases, wake-only taps, coordinate conversion, and
palette parsing. Provider tests cover disabled selection, drag-off, wake-only,
and credential removal during a touch. Rune headless tests inspect actual element
geometry and logo coverage; integration tests render the real scene in all modes.
The credential helper tests verify fail-closed behavior and boolean-only output.
Volume tests use a mock `wpctl` to check write coalescing, mute, external changes,
clamping, unavailable audio and cancellation. Simulated drag tests exercise the real
controller, and preview tests verify that no audio commands are issued.
Simulator tests cover proportional coordinates, actual window mouse clicks/drags,
focus-loss releases, monitor/argument handling, graceful termination, and isolation
from the physical test-state file and real audio.
Tests never change real audio levels or inject keys into your desktop. DRM activation, raw touch events, and keyboard delivery additionally
require a hardware smoke test under the compositor.

The vendored virtual-keyboard protocol is from
[wlroots](https://github.com/swaywm/wlroots/blob/master/protocol/virtual-keyboard-unstable-v1.xml)
and retains its upstream MIT copyright notice.
