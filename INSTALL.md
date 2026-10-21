# Installing podctl

podctl is distro-agnostic: a static-ish Rust binary that drives the
standard Linux stack (BlueZ, PipeWire/PulseAudio, D-Bus, systemd-user).
There is nothing distro-specific in the code — only the handful of CLI
tools it shells out to need to be present.

## Requirements

- A Linux kernel with Bluetooth + L2CAP (every mainstream kernel; the
  AAP link uses a raw `AF_BLUETOOTH` SEQPACKET socket on PSM 0x1001).
- BlueZ 5.x with `bluetoothctl` and a paired Bluetooth adapter.
- A D-Bus **system** bus (BlueZ) and, for the tray/popup, a **session**
  bus.
- For the audio verbs (`volume`, `mute`, `profile`, `codec`, …):
  PipeWire (with the PulseAudio shim) **or** PulseAudio — podctl talks to
  `pactl`. On Debian and Ubuntu `pipewire-pulse` does not bring `pactl`
  along: that one is in `pulseaudio-utils`.
- For `podctl meter` and the conversation-awareness volume stage:
  `pw-record` (pipewire-audio). Without it both fall back to `parec`
  (pulseaudio-utils), which reads the sink monitor — silent for
  Bluetooth sinks on PipeWire.
- To build from source: Rust ≥ 1.89 (edition 2024) and a C linker
  (`cc`/`gcc`).

Core control (battery, listening mode, conversation awareness,
connect/pair) needs only BlueZ + the kernel. Audio features degrade
cleanly with a clear message if `pactl` is absent; `podctl meter` says so
if neither `pw-record` nor `parec` is there.

## Runtime tools podctl invokes

| Tool | Package (typical) | Needed for |
| --- | --- | --- |
| `bluetoothctl` | bluez / bluez-utils | everything (device + AAP) |
| `dbus-send` | dbus | `podctl rename` |
| `pactl` | pulseaudio-utils (pipewire-pulse on Arch) | audio verbs |
| `pw-record` | pipewire-audio | `podctl meter`, conversation ducking |
| `parec` | pulseaudio-utils | fallback for the above on PulseAudio |
| `systemctl` | systemd | `podctl install`/`reboot` user services |

## Dependencies per distro

Build deps (`rust`, `cargo`, a linker) are only needed to compile; a
prebuilt binary needs just the runtime tools above.

**Arch / CachyOS / Manjaro**
```
sudo pacman -S --needed rust bluez-utils dbus
# Audio + meter:
#   - PipeWire systems: sudo pacman -S --needed pipewire-pulse pipewire-audio  (pactl + pw-record)
#   - PulseAudio systems: sudo pacman -S --needed libpulse pulseaudio
```

**Debian / Ubuntu / Mint**
```
sudo apt install cargo bluez dbus pipewire-pulse pipewire-audio pulseaudio-utils
# (pw-record comes with pipewire-audio; pulseaudio-utils provides pactl
#  and the parec fallback, same package on a PulseAudio box)
```

**Fedora**
```
sudo dnf install cargo bluez dbus pipewire-pulseaudio pipewire-utils pulseaudio-utils
```

**openSUSE**
```
sudo zypper install cargo bluez dbus-1 pipewire-pulseaudio pipewire-tools pulseaudio-utils
```

If your Rust is older than 1.89, install a current toolchain via
[rustup](https://rustup.rs) — distro Rust is often behind, and cargo
will otherwise stop with a clear `rust-version` error.

## Build and install

```
git clone https://github.com/Rockykln/podctl && cd podctl
cargo build --release
./target/release/podctl install            # core (CLI + daemon)
./target/release/podctl install --with-tray --with-popup
```

`podctl install` is interactive and needs no root. It copies the binaries
to `~/.local/bin/`, installs shell completion (bash/zsh/fish, picked
from `$SHELL`), man pages, and — if you accept — a systemd **user**
service for the daemon (and tray/popup with the flags). It is
idempotent; re-running it is safe, and services that are already
running are restarted so an upgrade takes effect. `podctl uninstall`
removes everything it created; `--purge` also removes your settings in
`~/.config/podctl` and the proximity keys in `~/.local/state/podctl`.

If `~/.local/bin` is not on `$PATH`, the installer prints the exact
line for your shell rc. Same for `MANPATH` and, for zsh, `fpath`.

**With a distro package installed** (`podctl-bin`, `podctl-git`),
`podctl install` stops: the copies in `~/.local/bin` would shadow
`/usr/bin/podctl` and the units in `~/.config/systemd/user` would take
precedence over the package's, so later `pacman -Syu` upgrades would
have no effect. Enable the packaged services instead —
`systemctl --user enable --now podctld` — or pass `--force`.

`podctl reboot` restarts the installed user services by hand, for
instance after replacing the binaries yourself.

### Prebuilt binaries

Each release has `podctl-<version>-<arch>-linux-musl.tar.gz` for
`x86_64` and `aarch64`, statically linked, with the same layout as the
repository's `dist/`. Next to each tarball are its `.sha256` and an
SBOM (`.cdx.json`, CycloneDX) of the crates built into it; the binaries
carry that list too, for `cargo audit bin`. GitHub attests where each
tarball was built — check it before unpacking:

```
gh attestation verify podctl-<version>-<arch>-linux-musl.tar.gz -R Rockykln/podctl
```

Unpacked, `./podctl install` sets it up as above.

## Optional components

`--with-tray` installs `podctl-tray`, a StatusNotifierItem. It needs a
tray host on the session bus:

| Desktop | Tray |
| --- | --- |
| KDE Plasma | native |
| Hyprland / sway / river + waybar | yes (`tray` module) |
| Xfce / MATE / LXQt | yes |
| GNOME | needs the *AppIndicator/KStatusNotifier* extension; `podctl tray status` says so |

The icon's menu starts with the battery line, then the four listening
modes, conversation awareness and connect / disconnect. Left click shows
the bubble (configurable as `left_click` in `~/.config/podctl/tray.toml`),
a double or middle click toggles noise cancellation against transparency,
and the wheel over the icon changes the volume in steps of five.

`--with-popup` installs `podctl-popup`, the case-open bubble. Backend is
auto-detected:

| Session | Popup backend |
| --- | --- |
| Wayland with `wlr-layer-shell` (Hyprland, sway, river, KDE Plasma, Wayfire) | full animated bubble |
| GNOME Wayland (no layer-shell) | notification fallback |
| X11 (i3, Xfce, MATE, …) | override-redirect window |

Everything about the bubble is tunable in `~/.config/podctl/popup.toml`
(the file is optional — these are the defaults):

```
enabled     = true      # false disables the bubble entirely
backend     = "auto"    # auto | wl | x11 | notify
theme       = "dark"    # dark | light
duration_ms = 5500      # time on screen, 500 … 60000
anim_ms     = 200       # slide in/out, 0 … 2000
output      = ""        # X11: monitor to centre on, e.g. "eDP-1"
```

`duration_ms` counts the hold only; the two slides add `2 × anim_ms` on
top. Out-of-range values are clamped rather than rejected.

On X11 the bubble is centred on a single monitor: `output` if set (names
as in `xrandr --listmonitors`), else the RandR primary, else the monitor
under the pointer. It is click-through, and without a compositing manager
(no picom/xcompmgr) the window is clipped to the card, since an X server
on its own ignores the alpha channel and would paint a black rectangle.

While the session is locked no bubble appears; `podctl popup` still shows
one on demand. Lock state comes from logind's `LockedHint`, with
`org.freedesktop.ScreenSaver` as the fallback — a desktop that reports
neither will keep popping bubbles behind its lock screen.

## The daemon's settings file

`~/.config/podctl/daemon.toml` is optional; the only setting so far is
the Bluetooth LE listener:

```
ble = true      # false stops the daemon listening for the case
```

While the buds are disconnected the daemon listens for the case's own
Bluetooth LE announcement, which is what makes the bubble appear as the
lid opens instead of three to five seconds later. It reads only
announcements that resolve against the key the buds handed out over the
connected link; that key stays in `~/.local/state/podctl/keys-<address>`
(mode `0600`) and is never sent anywhere. `podctl ble off` turns the listening off
and writes that setting; a running daemon notices within a few seconds.

## Compositors without systemd session integration

`podctl-tray` and `podctl-popup` install as systemd **user** units wanted
by `graphical-session.target`, and they need two things from the session
that not every compositor provides:

- **`graphical-session.target` has to be reached**, or the units are
  enabled but never autostart at the next login. `podctl install` starts
  them with `--now`, so the gap only shows up after a reboot.
- **The systemd user manager has to know `WAYLAND_DISPLAY` / `DISPLAY`.**
  Units inherit nothing from your shell. Without those variables the
  popup can't find the compositor and falls back to plain desktop
  notifications instead of the layer-shell bubble. With `backend = "auto"`
  the journal names the backend it fell back to; a backend you name
  yourself fails instead of falling back.

Plasma and GNOME do both for you. Hyprland, sway, river, Wayfire and
bare X11 sessions generally do not — add this once, early in the
compositor's startup (Hyprland `exec-once`, sway `exec`, i3 `exec`):

```
dbus-update-activation-environment --systemd WAYLAND_DISPLAY DISPLAY XDG_CURRENT_DESKTOP
systemctl --user start graphical-session.target
```

`podctl debug` reports both under `[desktop services]` — it flags a
variable that is set in your shell but missing from the systemd user
environment, which is the exact failure above.

## Multi-adapter hosts

If `/sys/class/bluetooth` lists more than one `hci*` device and the
default (first enumerated) is the wrong one for AirPods, pin the
adapter explicitly:

```
export PODCTL_ADAPTER=hci1
```

The variable is read by `podctl rename` (D-Bus path resolution) and by
any future code that needs an adapter id. Bluez itself still routes
based on which adapter holds the paired device, so `connect` /
`disconnect` aren't affected.

## Optional: tell BlueZ the host is an Apple device

A few AirPods features only light up when the bud believes it is paired
to a Mac. Find My is not among them — that one runs through iCloud and
stays out of reach on Linux. Linux can advertise itself as Apple's vendor ID
by adding a single line to `/etc/bluetooth/main.conf`:

```ini
[General]
DeviceID = bluetooth:004C:0000:0000
```

Then `sudo systemctl restart bluetooth` and re-pair. This is a
host-wide change — *every* Bluetooth device sees the new vendor ID, so
verify your other peripherals still pair correctly afterwards. podctl
works without it; this is purely for the extras.

Originally documented by [LibrePods](https://github.com/kavishdevar/librepods).

## Troubleshooting

- "pactl not found" → install `pulseaudio-utils` (on Arch: the `pactl`
  in `pipewire-pulse`), see the table above. Core control still works
  without it.
- "bluetoothctl … failed" → ensure `bluetooth.service` is running and
  the AirPods are paired (`podctl list`).
- Tray invisible on GNOME → install the AppIndicator extension; verify
  with `podctl tray status`.
- `man podctl` not found → the installer prints the `MANPATH` line if
  `~/.local/share/man` is outside your manpath.
- Conversation Awareness only reacts while audio is playing — that is
  the device's own behaviour, not a podctl limitation.
- Something is not working and the message does not say why → `podctl
  -v <command>` logs what the CLI does (which socket, which `pactl`
  call, what the daemon answered). For the services, `PODCTL_LOG=debug`
  and `journalctl --user -u podctld -f`; `podctl debug` has the last 40
  journal lines of all three at the end, redacted.
