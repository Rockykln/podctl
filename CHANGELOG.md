# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [SemVer](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- **Battery could stay empty for a whole session.** The AAP worker was
  started once per BlueZ connect edge and never checked again. If the
  L2CAP connect lost the race against the AirPods' own profile setup, or
  the device dropped the socket while the ACL link stayed up, the slot
  kept a dead thread and nothing ever restarted it — `podctl b`, the
  tray and the popup showed `—` until the user disconnected by hand.
  There is now a supervisor that reaps a finished worker and respawns it
  under a 2 s … 60 s backoff, plus five retries (~5 s) on the initial
  L2CAP connect itself.
- Battery watchdog: the AirPods push their state once after a subscribe
  and then only on change, so a lost dump left the daemon blind. A live
  link reporting no battery at all is now re-subscribed twice, 6 s apart,
  and `podctl status` / `podctl b` nudge the device and wait up to 0.9 s
  rather than answering with three em-dashes.
- **Wedged AAP socket.** Reproduced on a live Pro 2: the L2CAP connect
  succeeds, handshake and subscribe both write cleanly, and the device
  then sends nothing at all — not even the handshake reply — for as long
  as that socket stays open. Re-subscribing on it changes nothing; a
  fresh connection delivers the whole ~34-frame dump, battery included,
  within a second. So when re-subscribing doesn't help, the watchdog now
  recycles the socket (twice per connected period, then it stops poking).
- `podctl status` / `podctl b` say *why* the levels are missing —
  whether the AAP link is up and the device just hasn't reported, or
  there is no AAP link at all — instead of three bare dashes. `podctl
  debug` gained matching `bluez link` / `aap link` / `battery data`
  lines, and `aap_linked` is part of the daemon's state snapshot.
- The reconnect backoff in `podctl-popup` and `podctl-tray` ratcheted up
  permanently — it was never reset after a session that worked. A few
  daemon restarts were enough to put both on a fixed 30 s retry, so the
  popup missed lid events and the tray sat on "Not connected". Only a
  session that fails fast extends the delay now.
- Disconnecting clears `case_lid_open` along with the rest of the
  AAP-derived state. Leaving it at `Some(true)` swallowed the lid edge on
  the next connect, and with it the popup that edge triggers.
- The notification fallback backend hard-coded a 5 s expire timeout, so
  on GNOME Wayland the bubble vanished before the hold loop was done. It
  follows `duration_ms` now.

### Changed
- `podctl-popup` stays up for 6.5 s instead of 5 s. Five was too tight to
  read the rings and the mode line on a bubble you weren't already
  looking at. `duration_ms` in `~/.config/podctl/popup.toml` overrides
  it, and the popup config is documented in `INSTALL.md` for the first
  time.
- A battery frame that lands *after* the bubble is already on screen
  (rings still showing `—`) restarts the hold, so the real numbers get
  the full window instead of its remainder.
- `duration_ms` and `anim_ms` are clamped (500…60000 ms and 0…2000 ms)
  instead of being taken verbatim — `duration_ms = 0` used to make the
  bubble flash and vanish.
- Both man pages announced `podctl 0.1.0` in their `.TH` header through
  the whole 0.1.1 release. A test now asserts the header carries
  `CARGO_PKG_VERSION`, so it cannot drift again.
- `podctl version` was missing from all three shell completions, and
  `podctl.1` documented press-tone only under its `tone` alias. Tests
  now check every command against the man page and the completions.
- README claimed button presses "arrive unsolicited". They don't: no
  press opcode is confirmed, so `Event::Press` is never emitted and the
  `press counts` block in `podctl status` stays at zero. Documented as
  unimplemented alongside the other AAP gaps.

### Added
- `podctl debug` gained a `[desktop services]` section: whether
  `graphical-session.target` is active, the state of the three user
  units, and whether the systemd user manager actually carries
  `WAYLAND_DISPLAY` / `DISPLAY` — it flags a variable that is set in
  your shell but missing from the user environment. That mismatch is why
  the popup silently falls back to plain notifications on compositors
  without systemd session integration; `INSTALL.md` now documents the
  two lines that fix it.
- `CONTRIBUTING.md` notes that `rust-toolchain.toml` is only honoured
  with rustup — without it your distro Rust is used and local clippy
  results won't match CI either way.

### Changed
- `chunks_exact(N)` with constant `N` replaced by `as_chunks::<N>()` in
  the meter and the popup renderer. Equivalent, and it keeps clippy
  quiet on toolchains newer than the 1.89 pin.

## [0.1.1] - 2026-05-27

### Fixed
- AirPods 4 (ANC) Bluetooth product code is `0x201B`, not `0x2026`. The
  former mapping pointed at Beats Solo Buds; the new one is cross-checked
  against The Apple Wiki and OpenPods. AirPods 4 (non-ANC) keeps no code
  for now — no public PID has surfaced. `0x2025` (Beats Solo 4) and the
  old `0x2026` (Beats Solo Buds) now resolve to `Unknown` instead of being
  mis-identified as AirPods 4 variants.

### Added
- `AirPods Pro (3rd gen)` (`0x2027`) and `AirPods Max (2nd gen)`
  (`0x202D`) — model variants plus capability matrix entries.
- `PODCTL_ADAPTER=hciN` env override for multi-adapter hosts. Without it
  the first `hci*` from `/sys/class/bluetooth` is used, same as before.
- `podctl install` and `podctl uninstall` gracefully skip the systemd
  steps on hosts without `systemctl` (Artix, Devuan, Void, embedded
  rootfs), printing a clear note rather than failing.

### Changed
- All shell-outs to `pactl`, `bluetoothctl`, `dbus-send` and `systemctl`
  now run with `LC_ALL=C`. Defends parsers against gettext-translated
  output on non-English locales (e.g. German "ja"/"nein" instead of
  "yes"/"no" for `pactl get-sink-mute`).
- `INSTALL.md`: optional BlueZ `DeviceID` hint documented for users who
  want their host to advertise as an Apple device (some buds expose more
  features once they think they are paired to a Mac).

## [0.1.0] - 2026-05-20

First public release.

### Added
- `podctl` CLI with status, battery, listening modes, conversation awareness,
  ear detection, mic selection, one-bud ANC, AutoANC strength, chime,
  rename, connect / disconnect / pair / unpair / list / auto-connect.
- `podctld` daemon with Unix-socket IPC at `$XDG_RUNTIME_DIR/podctl.sock`
  (0600). Live `Event` stream via `podctl watch`.
- Standalone fallback: audio and BlueZ verbs work without the daemon.
- Apple Accessory Protocol over L2CAP PSM 0x1001, from scratch — no
  external bluetooth crate.
- `podctl-tray` (StatusNotifierItem) with battery tooltip and quick-action
  menu.
- `podctl-popup` case-open bubble with three backends (wlr-layer-shell,
  X11 override-redirect, GNOME notification fallback).
- `podctl install` / `podctl uninstall` — XDG-compliant user install, shell
  completions (bash/zsh/fish), man pages, optional systemd-user service.
- `podctl debug` with default DSGVO redaction (MAC OUI only, custom names
  masked, `$HOME` → `~`).
- `podctl meter` software RMS / peak dBFS meter via `parec`.

### Known limitations
- Spatial audio, loud-sound reduction, per-bud press actions and tone
  on press: AAP setting IDs not yet pinned down — the daemon returns
  a clear "not implemented for this device" error.
- Find My, Personalized Spatial Audio, Hearing Test and Announce
  Notifications via Siri are Apple-only and cannot be implemented on
  Linux.
