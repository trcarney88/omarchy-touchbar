# Rust Touch Bar hardware recovery

The Rust hardware backend is packaged as `touchmeter` 0.8.0 and has been
validated on Linux, including normal startup, rendering and touch interaction.
This runbook describes exceptional recovery behavior; its fault-injection
scenarios are not claimed as passed tests. Physical-panel support is Linux-only
and unavailable on macOS. Native macOS simulator validation has not been performed.

The Rust `touchmeter.service` user unit uses the graphical-session target. It
sets the Touch Bar brightness to 25% before launch and to 0 after stopping.
Ordinary `SIGTERM`/`SIGINT` requests safe quiescence. On a `FAULT_HELD` state,
the process intentionally does not acknowledge completion or drop resources:
there is no automatic recovery retry. Keep the unit/process alive until an
operator chooses a recovery action.

After installing this package and confirming the installed systemd supports
`systemctl --user kill`, an explicit `SIGUSR1` requests one bounded recovery
attempt with fresh barriers, then completes STOP (it does not resume the UI):

```sh
systemctl --user kill --kill-whom=main --signal=SIGUSR1 touchmeter.service
```

If that recovery cannot establish safe release, `SIGUSR2` explicitly requests
FORCED release. This may release unconfirmed keys or leave darkness unconfirmed;
it exits with status 78. Automatic restart is prevented for status 78. A stop or
restart after forced release must be an explicit operator action:

```sh
systemctl --user kill --kill-whom=main --signal=SIGUSR2 touchmeter.service
```

There is at most one automatic recovery attempt and one additional explicit
attempt per `SIGUSR1`; there is no automatic audio or function-key injection.
`SIGKILL`, supervisor termination, and kernel panic are outside these
guarantees. In those cases the inhibitor may remain held for a finite delay.

Rollback, only when authorized, uses the preserved verified package archive
(the archive is retained outside the repository):

```sh
sudo pacman -U pkgbuild/touchmeter-0.7.0-1-aarch64.pkg.tar.xz
systemctl --user daemon-reload
systemctl --user restart touchmeter.service
```

The archive SHA-256 is
`e8474b3dbcbe0aa6d9223a47efdc383c00a7848a6b7e77987669c276d17c7d6f`.
No install, rollback, or controlled hardware test has been performed as part
of this packaging preparation.
