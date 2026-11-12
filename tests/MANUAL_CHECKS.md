# Manual checks not represented by headless Rust tests

- Interactive simulator window behavior: launch the simulator in an interactive Wayland, X11, or macOS window session, verify movable/resizable window behavior and monitor selection, then close it and confirm clean process exit. The application uses its Rust UI; this is not a Qt check.
- Runtime window update lifecycle: while the simulator window is open, update and clear test fixtures and verify the visible window refreshes; verify orderly termination. Headless CLI, fixture, and preview behavior is covered by Rust tests, but visible window exposure and lifecycle are not.
- On macOS, manually verify simulator window behavior only. The current application does not support physical Touch Bar integration there.
- Physical-device integration, if required for a release, is a separate Linux check on supported Linux hardware; it was not exercised by these headless tests. Do not infer macOS physical-device support from simulator checks.
