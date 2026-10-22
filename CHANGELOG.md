# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [SemVer](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
Scripts and long-running clients need to adapt to the first six
entries when upgrading from 0.3.0.
- **`--json` has one shape for every command**:
  `{"schema":1,"ok":true,"kind":"state","data":{…}}`, one object on one
  line, so `watch` and `meter` stream newline-delimited JSON. There were
  three shapes before (`{"kind":…}`, `{"daemon":…}` for `version`,
  `{"ble":true}` for `ble`) and `debug --json` ignored the flag.
  `--pretty` indents it. Errors are an envelope with `ok: false`, the
  message and the hint, on standard output. Documented in the man page
  under JSON OUTPUT.
- **JSON spells values the way the command line takes them**, so
  `podctl mode "$(podctl status --json | jq -r .data.settings.mode)"`
  works; it used to print `noise_cancellation` or `always_left`, which
  the CLI rejects. The old spellings are still read back.
- **Exit codes tell failures apart.** Every failure from the daemon
  exited 69. Errors carry a kind now (`unavailable`, `unsupported`,
  `not_implemented`, `backend`, `internal`, `usage`), seen by the shell
  as 69, 1, 1, 71, 70 and 64, with or without `--json`, and in JSON as
  `data.code`. Surplus arguments, an option a verb does not take and
  `podctl help nope` are usage errors (64): `podctl rename Test AirPods`
  now says to quote the name instead of renaming to "Test".
- **The socket protocol is at 2**, so `podctl`, `podctld`, the tray and
  the bubble have to be the same version. A 0.3 daemon answers a 0.4
  `podctl mode anc` with "malformed request"; run `podctl reboot` after
  an upgrade.
- **`podctl status` was rewritten.** One label column with the command
  names as labels (`ear`, `loud-reduction`, `auto-connect`), one line
  per bud with a bar on a terminal and plain columns in a pipe, and the
  charging state as a word instead of a two-cell bolt. `trusted` is
  gone: it is the same BlueZ flag as `auto-connect`. Disconnected, it
  shows the levels the case last announced over bluetooth LE, with
  their age. `podctl battery` prints the same rows; `podctl list` cuts
  a long name instead of shifting the columns.
- **`podctl pair` pairs what you name.** It used to pair and trust the
  first unpaired device that looked like AirPods. It lists what it found
  and pairs the address you give, or the only candidate with `--yes`.
- **`podctl auto-anc` takes `less-noise`, `default` or `more-noise`**,
  the slider under Adaptive Audio on iOS, and switches to
  Adaptive first, the one mode it acts in. The buds keep only these
  three stops; a number still works and is rounded as they round it.
  The help had the direction backwards: 100 lets the least noise in.
- **Error messages have one shape**: `podctl: mode: unknown value 'foo'
  (expected off, anc, transparency, adaptive)` with `hint:` on the next
  line, and the daemon's own errors name the command too
  (`podctl: connect: …`). A mistyped command names the one you meant,
  `pending` answers print `pending: <reason>` instead of
  `ok (pending: …)`, and `meter --interval abc` or `0` is a usage error
  instead of a silent 100 ms or a busy loop.
- **The command line comes from one table** (`src/commands.rs`). Help,
  the three completion scripts, argument checks and the man page were
  hand-kept copies that had drifted: `podctl m <TAB>` and `podctl
  --json mode <TAB>` completed nothing, the internal `debug
  emit-case-lid` was offered to everyone. `podctl completion <shell>`
  writes the script from the table, the bash one no longer needs the
  bash-completion package, and a test holds `dist/` and the man page to
  it.
- `PODS_NO_LR` is now `PODCTL_NO_LR` (the old name still works).
- **The case-open bubble is redrawn.** Each model is drawn as it looks,
  buds and case on one millimetre scale (AirPods Max on its own), with
  the finger notch on every Pro case. The title is the name you gave
  the buds, cut with an ellipsis; the line under it is the listening
  mode or "Connecting…". Both buds share one value when within 5
  points; red is at or below 20 %, green charging, and the red frame is
  gone. `anim_ms` in `popup.toml` is no longer read.
- **The bubble is sharp on scaled screens.** On Wayland it draws at the
  scale of the output it lands on, fractional scales such as 1.25 or
  1.75 included (`fractional-scale-v1` with `viewporter`); on X11 at
  `Xft.dpi` / 96. It used to draw at 1× and leave the stretching to the
  compositor. Shadows are blurred only where they fall: a card at 2×
  takes 0.3 s instead of 1.7 s, and a bubble that stands still no
  longer pushes sixty identical frames a second.
- **The bubble moves in time.** It rises into place on a spring while it
  fades in, the buds hop once, the rings fill in turn, and changes while
  it is up (a ring sliding to a new level, a case put on charge, the
  shared value splitting into left and right) play out too. Where
  nothing can blend (notification fallback, X without a compositor) the
  card simply appears. `podctl-popup --demo --updates` shows it.
- **The bubble answers to the pointer.** A click closes it; while the
  pointer rests on it the bubble stays, and at least 1.5 s remain after
  it leaves. The close button lights up and the pointer turns into a
  hand. Only the card takes clicks, the shadow lets them through (on
  X11 the whole bubble used to be click-through). `output` in
  `popup.toml` works on Wayland too; on X11 the bubble goes to the
  monitor under the pointer before the primary one.
- **The tray icon is its own**: both buds over a level bar, red at 20 %
  and below, a bolt while charging, faded when not connected. The
  theme's headphones looked the same either way and swapped to a laptop
  battery glyph at low levels. The icons are written to the tray's
  runtime directory at start, so nothing new has to be installed;
  `podctl-tray --demo` shows them. On GNOME they are drawn bolder for
  the 16 px panel.
- **The tray menu is rebuilt.** Name and model on top, a line per bud
  and the case, the modes in iPhone order, submenus for volume, audio
  output and the microphone. What the model cannot do is hidden, not
  greyed out; not connected it offers Connect first and the last levels
  the case announced. The double click is gone, because it ran the
  single-click action first; the middle click toggles the mode.
- **The tray reports failures.** A tray whose daemon is gone no longer
  looks like disconnected AirPods: the menu says podctld is not running
  and offers to start it, and every failing action says why in a
  notification. Scrolling steps from a fresh reading and adds touchpad
  steps up to one notch instead of jumping to 0 or 100. The menu waits
  at most 300 ms for a busy daemon (it could wait 10 s). The icon comes
  back after a panel restart, and a missing tray host no longer sends
  the tray into a restart loop. `podctl tray status` named `mode-cycle`
  as the default left click; it is `popup`.
- The low-battery warning is per bud and comes in two steps (20 % and
  10 %, critical), names the bud, replaces the previous one and does not
  return when the other bud goes into the case for a moment. The tray's
  red state no longer stays off for a low bud while only the case
  charges. The tray unit gets its own runtime directory and a tighter
  sandbox (`ProtectHome=read-only`, `MemoryDenyWriteExecute`, Unix
  sockets only).
- The duck for speech starts as soon as the AirPods report it, by the
  smallest step, while the music's level is measured; about 0.7 s
  earlier than before.
- **Release binaries are stripped**: 79 MB down to 13.5 MB for all four
  together. `fontdue` and `thiserror` are no longer dependencies.

### Added
- **`podctl mode next` and `podctl volume +5` / `-5`**, for hotkeys that
  cannot know where the AirPods are now. `next` goes round the modes the
  model has and leaves off out, as a press of the stem does; a volume
  step stops at 0 and 100. The README has bindings for Hyprland, sway
  and Plasma.
- **Apple's model number** (`A3047`) in `podctl status`, `podctl debug`
  and as `info.model_number` in JSON, read from the device info the buds
  send on connect. Their serial numbers are in there too and are left
  alone.
- **Release tarballs for aarch64** (Asahi Linux, ARM laptops) next to
  x86_64, each with a build attestation from GitHub
  (`gh attestation verify`), a CycloneDX SBOM, and the dependency list
  embedded in the binaries by `cargo auditable`. A release is only built
  once the tests pass and the tag matches `Cargo.toml`.
- **The daemon introduces itself.** `podctl version` and `podctl debug`
  show which `podctld` is on the socket and which protocol it speaks.
  When that is not the binary's own, every command that runs into it
  says so: *"podctld is 0.3.0 (protocol 1), this is 0.4.0 (protocol 2)
  — restart the services with 'podctl reboot'"*. The tray notifies, the
  bubble logs it. Before, a tray or CLI left running across an upgrade
  just stopped working.
- **A `watch` stream opens with the current state**, so a subscriber
  needs no second connection for it. If the daemon has to drop events
  for a client that cannot keep up, it sends the state again.
- **Music that stopped with the link comes back with a bud.** When the
  AirPods disconnect while something plays, a bud put in within 5
  minutes after the reconnect resumes it — not on the reconnect itself,
  since opening the case connects them while still in a hand. Music
  paused before stays paused.
- **`podctl -v`** logs what the CLI itself does on stderr: which socket
  it tried, what it asked the daemon, which `pactl` calls it ran.
- **`PODCTL_LOG`** sets the level for every podctl process and takes
  precedence over `RUST_LOG`.
- **`podctl debug` carries the logs.** The report ends with the last 40
  journal lines of `podctld`, the tray and the bubble, redacted like the
  rest (host name, user name, serial numbers, MAC addresses). It also
  names the firmware of both buds and the case, reads capabilities from
  the daemon instead of guessing from the modalias, lists `tone`,
  `press-vol` and `ear-det-setting`, and says when the systemd user
  manager cannot be reached (under `sudo`, over ssh without lingering,
  in containers) instead of reporting every variable as unset.
- **`theme = "auto"`** in `popup.toml`, now the default besides `dark`
  and `light`: the bubble follows the desktop's light or dark style via
  the portal and picks up a change from the next bubble. Without a
  portal it stays dark.
- **`motion`** in `popup.toml` (`auto`, `full`, `reduced`). `auto`
  follows GNOME's animations switch, KDE's animation speed and the
  portal's reduced-motion key; reduced motion only fades.
- **The bubble shows on GNOME as itself.** GNOME lets no program float a
  window, so it used to fall back to XWayland or a plain notification.
  A shell extension for GNOME 46–51 (`contrib/gnome-shell-extension/`)
  now shows the frames podctl-popup draws, at the monitor's scale, with
  hover and click, and holds back GNOME's volume OSD meanwhile.
  `podctl install` offers it, `podctl install --gnome-extension`
  switches it on alone (the way for a distro package, whose release
  archive carries it), `backend = "gnome"` picks it by name, and `podctl
  debug` says whether it runs.
- `podctl-popup --dump` takes `--model`, `--scale` and `--backdrop`,
  `--demo` takes `--hold` and `--motion`; a test compares all 28
  renders with the reference images in `tests/golden`.

### Fixed
- **`podctl pair` could leave a pairing that did not last, and failed
  through the daemon.** On an adapter that is not pairable (Debian's
  default) BlueZ paired without a key and forgot the AirPods at the next
  disconnect; pair now makes the adapter pairable for the moment and
  says so when BlueZ still kept no key. Through a running daemon it
  always failed with "Resource temporarily unavailable (os error 11)":
  the scan takes twelve seconds, the CLI gave up after ten. Already
  paired AirPods are named as such instead of "no new AirPods spotted".
- **A connect that BlueZ turns down says what to do**:
  "br-connection-page-timeout" reads as the AirPods not answering, with
  the hint to open the case. `connect` and other BlueZ commands used to
  fail with an empty reason, because `bluetoothctl` prints it on stdout.
- **Paired AirPods could be missing from `podctl list` and `unpair`**
  until their first connect after the adapter came up (no Modalias yet);
  they are known by name until then. BlueZ 5.86 and later also print
  some lines of `bluetoothctl devices` in grey, and those devices were
  skipped altogether: in `list`, and in `pair`, which then did not see
  that the AirPods were already paired.
- **The bubble could stay up for good** while the AirPods reconnected or
  paired: every repeated event restarted its time. Only a real change
  shows it now; a resting pointer holds it for 30 s at most, and on X11
  it checks the real pointer position when the leave event never came.
  Buds that connect for a moment in a case just shut no longer bring it
  back, and the mode they report on connecting is not taken for a
  change.
  It could also paint into a buffer the compositor still read, and on a
  surface nobody reads it could no longer be hidden or quit.
- **The notification fallback was empty and never showed on GNOME.**
  It was sent as low urgency, which GNOME files straight into the
  message tray, and the rendered card was squeezed into a cropped
  square. It is now the device name, the levels as text and the
  headphones icon.
- **`podctl reboot` started services stopped on purpose**, which next to
  a daemon run some other way meant a second daemon. It restarts what
  runs or has failed and names what it left alone.
- **`podctl status` kept showing battery, in-ear state and settings
  after the AirPods disconnected**, for up to 3 s or longer. Only what
  podctl stores itself (ear detection, microphone) stays.
- **The duck for speech ended at the wrong level.** Speaking again
  while the volume came back up started a second duck from the halfway
  level; the same happened during the restore after a crash. A volume
  lowered for speech also stayed down after a dropped link, SIGTERM or
  a crash, because with the AirPods' sink gone it looked like a change
  of your own. It is now noted on disk and restored on start and on
  reconnect, new speech stops a running restore, and a volume you move
  yourself stays put. The watchdog for a missing end of speech waits
  30 s instead of 10: the buds send nothing while someone keeps talking.
- **Battery, in-ear and ear detection went quiet after using the
  iPhone.** The AirPods end the AAP session from their side and wait
  for a new handshake; podctl kept listening on the dead socket. It
  opens a new one.
- **The music resumed while a bud was being taken out**, because the
  sensor reports "in" again while the bud is still in a hand. After a
  pause the buds must stay in for 2 s; putting one back in later takes
  0.5 s, half of before.
- **Audio went to the first Bluetooth device, not the AirPods.** With a
  speaker or headset also paired, volume, mute, profile, codec, default
  sink, latency, `meter` and the ducking could land on that one, and a
  bud coming out paused whatever played there.
- **`podctl profile off` was a dead end**: the card was looked up via
  its sink, which that profile lacks, so profile, codec and latency
  answered "no AirPods card found" from then on.
- **`podctl status` named the wrong codec.** Plain `a2dp-sink` was read
  as AAC, but a PipeWire without AAC runs SBC; the sink is asked now,
  and `podctl codec` lists what is really on offer.
- **A missing `pactl` was reported as missing AirPods.** On Debian and
  Ubuntu `pipewire-pulse` does not bring `pactl` along; the install
  notes got that wrong too.
- **A settings frame without a value switched the microphone over.** A
  missing byte was read as zero, which the mic selection calls "auto";
  the same frame set the chime volume to silent and the adaptive-ANC
  strength to 0. Such frames are ignored, and one table (`aap::wire`)
  now maps bytes and values in both directions, with round-trip tests.
- **Ear detection and microphone mode showed as nothing** until set in
  that session, since the Pro 2 USB-C leave both out of their reported
  settings. Both live in `~/.config/podctl/daemon.toml` now, so the
  ear-detection switch survives a restart. Buds that do report them
  still have the last word.
- **The daemon could be held open by a silent client.** A connection has
  five seconds to send its request, a request may be 64 KB (an endless
  line used to buffer until the daemon was killed), and sixty-four
  connections are served at once. A `watch` client that went away left
  its registration behind, so `podctl popup` reported success with
  nobody to draw the bubble. The daemon also kept the number of the
  socket to the buds rather than the socket, so the kernel could hand it
  to a `watch` stream or BLE scan that then ended for no visible reason.
- **Nothing on the way to the buds had a timeout**: a connect to buds
  out of range held the poll loop for the kernel's own timeout, reads
  blocked likewise, and a write held the socket's lock. The socket was
  also inherited by every process the daemon started.
- **Every log line under systemd carried two timestamps** and escape
  codes journald does not need. The three services share one format and
  leave timestamp and colours to journald when stderr really is the
  journal (checked by inode via `JOURNAL_STREAM`, since a terminal
  started from a unit inherits the variable).
- **The identity resolving key of the buds was written to the debug
  log**, and device names reached the terminal unfiltered, escape
  sequences and bidi overrides included. Both are fixed.
- **`podctl install` left the old version running, shadowed a distro
  package and replaced binaries non-atomically.** After an upgrade the
  services kept running from the deleted binary; the installer restarts
  what runs. With `podctl-bin` or `podctl-git` installed, the copies in
  `~/.local/bin` and the per-user units took precedence, making every
  `pacman -Syu` pointless; it now stops and points at the packaged
  services, or installs anyway with `--force`. A binary is written
  beside the target and renamed over it, and man page write failures are
  no longer reported as success.
- **`podctl uninstall` could hand the wrong package to pacman.** It
  asked who owns `/usr/bin/podctl` while running from `/usr/local/bin`;
  it asks who owns the running copy. Proximity keys and settings are
  named in the listing, and `--purge` removes them.
- **The no-daemon marker was written where the CLI does not look**
  (always `~/.config`, never `XDG_CONFIG_HOME`), so the "running without
  daemon" banner kept appearing. All paths come from one place now.
- The zsh completion went to a directory outside a default `fpath`,
  named `_pods`. It is `_podctl`, the installer prints the `fpath` line
  when needed, and `uninstall` still removes the old file.
- **`--help` ran the command** for `debug`, `meter`, `completion` and
  `version`. `podctl ping` answered "pong" without a daemon and exited
  0. `meter --json` never printed JSON, and its first reading was
  implausible (the capture buffer's leftovers).
- A BLE battery reading above ten tenths was reported as up to 140 %.
- `podctld.service` ordered itself against `bluetooth.service`, which a
  user unit cannot do, and messages pointed at `podctl setup`, which
  never existed.
- Documentation: the four settings whose AAP ids are unknown (`spatial`,
  `loud-reduction`, `press`, `tone-on-press`) are marked as not
  implemented in the overview, help pages and man page; loud-sound
  reduction lists its models; AirPods 4 non-ANC has a known product
  code; the prebuilt tarball is mentioned in the install notes.

## [0.3.0] - 2026-09-24

### Added
- **The bubble appears as the case opens.** It used to wait for the
  classic Bluetooth link, three to five seconds later, by which time a
  bud was usually already in an ear. While the buds are disconnected the
  daemon now listens for the case's own Bluetooth LE announcement and
  shows the bubble, with the battery levels it carries, right away.
  Announcements from other Apple devices are dropped: only an address
  that resolves against the buds' own identity resolving key is read.
  The key is asked for once over the connected link and kept in
  `~/.local/state/podctl/keys-<address>` with mode `0600`.
- **`podctl ble [on|off]`** turns that listening on or off, and without
  an argument says which it is. It writes `~/.config/podctl/daemon.toml`,
  the daemon's new settings file, which can be edited by hand as well; a
  running daemon picks a change up within a few seconds.
- **`podctl debug` reports the listener**, whether it is on and whether
  the proximity key has been stored.
- **The tray menu leads with the battery** — `L 92% · R 84% · Case 88%`
  as its first line, where it used to take a hover over the icon to see
  it at all.
- **The four listening modes sit in the menu itself**, not behind a
  `Mode` submenu that cost a second click for the one thing the icon
  gets opened for.
- **`Connect` when the buds are away.** The menu only ever offered
  `Disconnect`, greyed out while they were gone.
- **Middle click or double click on the icon** switches between noise
  cancellation and transparency, and **the wheel over the icon** changes
  the volume in steps of five.
- **The tray tooltip names the mode** (`AirPods Pro — Transparency`) and,
  while the buds are away, shows the levels the case announces instead of
  a flat "Not connected".
- **No bubble while the session is locked.** A lid opened behind a lock
  screen used to pop the bubble anyway — either hidden under the lock,
  or, on X11, on top of it. `podctl popup` still shows one when you ask
  for it by hand.
- **`output` in `popup.toml` pins the X11 popup to one monitor.** The
  bubble used to be centred on the whole X screen, which on two side-by-side
  monitors put it across the seam. It now centres on the configured output,
  else the RandR primary, else the monitor under the pointer.

### Changed
- **The bubble goes away a second sooner**, 5.5 s instead of 6.5 s. It
  now opens as the lid does rather than seconds later, and at that point
  it was outstaying its welcome. `duration_ms` in `popup.toml` still
  overrides it.

### Fixed
- **The bubble could show a state a quarter of an hour old.** It asked
  the daemon for one snapshot when it started and then followed events —
  so a bubble whose process attached while the daemon was still coming
  up stayed on "not connected" for its whole life, and said "In case"
  with the buds in your ears. It now takes a fresh snapshot whenever
  something puts it on screen.
- **A low battery had no icon of its own.** The tray has been reporting
  the "needs attention" state for a while, but never named the icon that
  goes with it, so nothing changed on screen.
- **Auto-pause could arrive up to fifteen seconds late.** Putting a bud
  back in makes the daemon wait for the AirPods sink to come back, and
  it held the lock on the player list while it waited — so taking a bud
  out again, and the ducking on speech, queued up behind that wait. It
  also resumed playback afterwards even when a bud had come back out in
  the meantime.
- **The X11 bubble swallowed clicks.** It reacted to none of them, so a
  click anywhere in its rectangle — including the transparent margin —
  simply went nowhere. It now carries an empty input region and clicks
  reach the window underneath.
- **The X11 bubble was a black rectangle without a compositing manager.**
  Alpha only means something while something composites; on a bare WM the
  server paints the raw pixels. The window is clipped to the card's
  opaque area when no compositor owns `_NET_WM_CM_S<n>`.
- **`podctl-popup --demo --backend notify` crashed** with "Cannot start a
  runtime from within a runtime" instead of showing the notification.
- **A backend named by hand fell back silently.** `--backend x11` or
  `backend = "x11"` answered with a desktop notification when X was
  unreachable, while the log still named x11 — the reason the systemd
  unit's private `/tmp` went unnoticed for so long. Named backends now
  fail with the error; only `auto` walks the chain, and the log says
  which backend actually opened.
- The X11 window announces itself as a notification rather than a utility
  window, picks a true-colour visual explicitly, and swaps pixel bytes on
  servers that ask for the other order.
- **`podctl meter` only ever showed silence on PipeWire.** It read the
  sink's pulse `.monitor` source with `parec`, which stays at -120 dBFS
  for Bluetooth sinks there. It captures the sink with `pw-record` now,
  the same way the conversation-awareness volume stage does, and falls
  back to `parec` where `pw-record` is missing. `--device` takes the sink
  name instead of the monitor source.
- **Parking one bud in the case stopped the music.** Putting a bud away
  to charge while the other stays in an ear is listening on one bud, not
  a break: playback now resumes once the bud is in the case. Taking the
  remaining one out still pauses.
- **The X11 popup fell back to a plain notification when run as a
  service.** `PrivateTmp=true` in `podctl-popup.service` hid the X
  server's socket in `/tmp/.X11-unix`, so the override-redirect window
  never opened and the notification fallback took over silently. The
  unit no longer uses a private `/tmp`.

## [0.2.0] - 2026-09-19

### Added
- **Auto-pause on ear removal.** Taking a bud out pauses the MPRIS
  player that is playing; putting it back resumes it, as on an iPhone.
  Only players `podctld` paused are resumed, only within 15 minutes, and
  only once the AirPods are the default output again — so buds coming
  back out of the case resume the music instead of starting it on the
  speakers. Players that mirror another one (browser integration,
  scrobblers) are left alone, since some of them turn Pause into a
  toggle and would undo it. The in-ear sensor flickers while a bud is
  pulled out, so playback resumes only after the bud has stayed in for
  a second. Follows the `podctl ear` setting.
- **Volume ducking for conversation awareness.** The buds only report
  that you are talking; the volume change was left to the host and
  never happened. `podctld` now measures how loud the music actually is
  (what plays times the sink volume) and fades it down to a level you
  can talk over — a little for quiet music, more for loud music,
  never to silence — and fades back up once you have stopped talking
  for a moment. Nothing moves while no audio is playing. Follows the
  `podctl conv` setting.

### Changed
- **`podctl watch --json`: mode and conversation events carry a named
  value.** They were serialised as `{"event":"mode","transparency":null}`;
  now `{"event":"mode","mode":"transparency"}` and
  `{"event":"conv_awareness","conv":"on"}`. Scripts matching the old
  shape need updating. The `{"kind":"done"}` subscription ack is no
  longer printed.
- Raw AAP frames are logged at debug level instead of info; the journal
  no longer fills with a line per frame.

### Fixed
- **The lid-closed and in-case events repeated.** The BlueZ poll every
  3 s replaced the cached state and dropped the lid flag, so the next
  battery frame announced the same lid edge again. The in-ear event
  fired on every frame, changed or not.
- **The bubble popped up when a bud went back in.** The AirPods switch
  mode by themselves on insertion and removal; the popup treated that
  like a mode change you made. Mode changes within 3 s of an in-ear
  change no longer show it.
- **The bubble opened halfway and vanished.** After every connect the
  lid state is unknown; the first battery frame without the case then
  counted as a "lid closed" edge and hid the bubble the connect had just
  shown. A daemon restart also announced the existing link as a new
  connection.
- `podctl help watch` listed stem-press events that are never sent, and
  its shell example matched output `watch` does not produce.

## [0.1.3] - 2026-08-29

### Fixed
- **A left-click on the tray icon did nothing, and said it worked.**
  The click asks the daemon to broadcast `ShowPopup`, which only
  `podctl-popup` draws. With that service down the event went to nobody
  and `podctl popup` still answered `ok` with exit 0 — no error, no log
  line, nothing to go on. Watch connections now declare a role, the
  daemon tracks whether a popup is attached, and refuses the request
  with the command to start it when none is. All three binaries have to
  be on the same version for this: an older `podctl-popup` attaches as a
  plain observer.
- **`podctl-popup` initialised no logging at all.** `podctld` and
  `podctl-tray` both set up a subscriber; the popup never did, so its
  unit's `RUST_LOG=info` went nowhere and the journal held nothing but
  systemd's own "Started" line — a bubble that never appeared left no
  trace. It logs its backend, theme and hold on startup, and the
  show/exit paths at debug.
- **`uninstall` stopped services it did not remove.** It ran
  `systemctl --user disable --now` on `podctld`, `podctl-tray` and
  `podctl-popup` unconditionally, including the units shipped by the
  `podctl-bin` / `podctl-git` packages. Removing a `~/.local` install on
  a machine that also had the package left the packaged tray and popup
  stopped and disabled, with nothing in the output saying so. Only units
  whose files the same run deletes are touched now.
- **A distro package went undetected once `podctl install` had run.**
  Package detection bailed out unless the *running* binary sat in
  `/usr/bin`, and `~/.local/bin/podctl` shadows it in `$PATH`. The
  package is now found either way: removed as before when it is the copy
  you invoked, otherwise listed under `left alone:` with the `pacman -R`
  line to remove it yourself.
- `podctl tray start` only started the service; the help had promised
  "enable + start" since the verb existed, so a tray started this way was
  gone after the next login. `start` enables, `stop` disables.
- `podctl tray status` reported a tray that was never installed as
  `inactive`, which reads like it is one `start` away. It says
  `not installed` and points at `--with-tray`.
- `uninstall` removed the completion script for `$SHELL` only, so
  uninstalling from a different shell than you installed from stranded
  the first one — on the machine this was found on, a fish completion
  survived a full uninstall. It sweeps bash, zsh and fish.

### Changed
- `podctl install` offers the tray icon and the case-open popup when
  their binaries sit next to `podctl`, instead of silently skipping them
  without `--with-tray` / `--with-popup`. Skipping the popup by default
  is what left the tray's own left-click with nothing to draw it.
  `--no-tray` / `--no-popup` skip the questions, the `--with-` flags
  still take them without asking, and `--yes` accepts both.

## [0.1.2] - 2026-08-27

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
