# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [SemVer](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Release tarballs for aarch64** (Asahi Linux, ARM laptops) next to
  x86_64, each with a build attestation from GitHub
  (`gh attestation verify`), a CycloneDX SBOM, and the dependency list
  embedded in the binaries by `cargo auditable`. A release is only built
  once the tests pass and the tag matches `Cargo.toml`.
- **Music that stopped with the link comes back with a bud.** When the
  AirPods disconnect while something plays through them, a bud in ear
  after the reconnect resumes it, as when one goes back in after being
  taken out — within 5 minutes, and not on the reconnect itself: opening
  the case connects them while they are still in a hand. Music paused
  before the link dropped stays paused.
- **`podctl debug` carries the logs.** The report ends with the last 40
  journal lines of `podctld`, the tray and the bubble, through the same
  redaction as the rest: the host name, the user name, serial numbers
  and MAC addresses are taken out of them. It also names the firmware
  of both buds and the case, reads the capabilities from the daemon
  instead of guessing them from the Bluetooth modalias, and lists
  `tone`, `press-vol` and `ear-det-setting`, which were missing.
- **`podctl -v`** logs what the CLI itself does on stderr — which
  socket it tried and what the error was, what it asked the daemon and
  what came back, which `pactl` calls it ran. Until now the CLI said
  nothing about its own work, and "it is not talking to the daemon" had
  no trail at all.
- **`PODCTL_LOG`** sets the level for every podctl process and takes
  precedence over `RUST_LOG`, so a `RUST_LOG=trace` left behind in a
  shell no longer makes everything started from it chatty.
- **The daemon introduces itself.** `podctl version` and `podctl debug`
  show which `podctld` is on the socket and which protocol it speaks,
  and when that is not the one the binary belongs to, every command
  that runs into it says so: *"podctld is 0.3.0 (protocol 1), this is
  0.4.0 (protocol 2) — restart the services with 'podctl reboot'"*. The
  tray says it as a notification, the bubble in its log. Before this, a
  tray or CLI left running across an upgrade simply stopped working,
  with "malformed request" as the only clue.
- **A `watch` stream opens with the current state**, so a subscriber no
  longer needs a second connection for it — an event could slip through
  between the two. If the daemon has to drop events for a client that
  cannot keep up, it sends the state again instead of leaving a gap.

### Fixed
- **`podctl pair` could leave a pairing that did not last.** On an
  adapter that is not pairable — Debian's default — BlueZ paired without
  keeping a key, and the AirPods were forgotten at the next disconnect.
  pair now makes the adapter pairable for the moment, puts it back, and
  says so when BlueZ still kept no key instead of reporting success.
- **The bubble could stay up for good** while the AirPods reconnected or
  paired: every repeated mode or connect event started its time over.
  Only a real change shows it now. A pointer resting on it holds it for
  30 s at most, and on X11 the bubble checks where the pointer really is
  when the leave event never came. Showing and hiding are logged with
  their reason.
- `podctl pair` says AirPods are already paired instead of "no new
  AirPods spotted", asks back with exit code 64 instead of 69 when it
  needs to be told which, and announces the 15 s it really takes.
- Paired AirPods that had not connected since the adapter came up could
  be missing from `podctl list` and `unpair`: BlueZ gives them no
  Modalias yet, so they are known by name until it does.
- **A tray whose daemon was gone looked like disconnected AirPods**, and
  a click on Connect failed without a word. The menu now says podctld is
  not running and offers to start it, and every action from the icon or
  the menu that fails says why in a notification.
- A double click on the tray icon ran the single-click action first, so
  it opened the bubble and changed the mode. The double click is gone;
  the middle click toggles the mode.
- Scrolling on the tray icon stepped from a volume that could be long
  out of date, and a touchpad sent it to 0 or 100 in one swipe. Small
  steps now add up to one notch, and a new scroll starts from a fresh
  reading.
- Opening the tray menu could wait up to 10 s for a busy daemon; it
  waits 300 ms at most and shows what it has.
- The tray's red "needs attention" state stayed off for a low bud while
  only the case was charging.
- `podctl tray status` named `mode-cycle` as the default left click; it
  is `popup`. It now reads the file with the tray's own parser.
- **`podctl reboot` started services that had been stopped on purpose.**
  It restarted every installed unit, and `restart` starts a stopped one
  too — next to a daemon run some other way, that is a second daemon.
  It restarts what runs or has failed and names what it left alone.
- **`podctl status` kept showing battery, in-ear state, listening mode
  and settings after the AirPods disconnected.** The link to the buds
  breaks a moment before BlueZ reports the disconnect, and the clean-up
  that follows only ran while the link task was still around, and it
  waited for the next poll of BlueZ, up to 3 s. Now only what podctl
  stores itself (ear detection, microphone) is left, a moment after the
  link breaks.
- **The duck for speech ended up at the wrong level.** Speaking again
  while the volume came back up started a second duck from that halfway
  level, so it never returned to where it was; the same happened during
  the restore after a crash. New speech now stops the restore and goes
  back down, and the level from before is kept. A link that dropped
  while the music was down lost the duck as well: with the AirPods'
  sink gone it looked like a volume change of your own. The note stays
  on disk then, and the volume is restored once they are back.
- The buds send nothing while someone keeps talking, so the watchdog
  for a missing end of speech now waits 30 s instead of 10 — it brought
  the music back up during a long sentence.
- **Battery, in-ear and the ear detection went quiet after using the
  iPhone.** Every so often the AirPods end the AAP session from their
  side and wait for a new handshake; podctl kept listening on the dead
  socket until the next reconnect. It opens a new one now.
- **The music resumed while a bud was being taken out.** The in-ear
  sensor reports "in" again for a second or more while the bud is still
  in a hand. Right after a pause the buds have to stay in for 2 s before
  playback resumes; putting one back in later takes 0.5 s, half of
  what it took before.
- `podctl connect` and the other BlueZ commands failed with an empty
  reason; `bluetoothctl` prints it on stdout.
- `podctl status` without a daemon ran `bluetoothctl` twice, once for
  the device and once more for the address the audio lookup needed.
- **A settings frame without a value switched the microphone over.** A
  missing byte was read as zero, and zero is exactly what the mic
  selection calls "auto"; the same frame also set the chime volume to
  silent and the adaptive-ANC strength to 0. Such a frame is ignored
  now. Which byte stands for which value is one table for both
  directions (`aap::wire`), with round-trip tests — reading a dump and
  writing a setting used to spell it out in two different files.
- **Every log line under systemd carried two timestamps**, ours and
  journald's, and escape codes journald does not need. The three
  services had a subscriber each with its own idea of the format; they
  share one now, which leaves the timestamp and the colours to journald
  when stderr really is the journal (the inode from `JOURNAL_STREAM`,
  not just the variable, which a terminal started from a unit inherits).
- **`podctl debug` blamed the session for an unreachable systemd user
  manager.** With no answer from it, every variable looked unset and
  the report printed *"set here but NOT in the systemd user env"* —
  under `sudo`, over ssh without lingering and in containers. It says
  the manager could not be reached.
- The three shell completions offered only the flags someone had
  written into them by hand; they come from the table, so `--verbose`
  is there too.
- **`podctl install` left the old version running.** `enable --now`
  does nothing to a unit that is already up, so after an upgrade
  `podctld`, the tray and the bubble kept running from the binary that
  had just been deleted until someone found `podctl reboot`. The
  installer restarts what is running.
- **`podctl install` shadowed a distro package without a word.** With
  `podctl-bin` or `podctl-git` installed, the copies in `~/.local/bin`
  take precedence in `$PATH` and the per-user units take precedence
  over the package's, so every later `pacman -Syu` had no effect. It
  stops now and points at the packaged services, or installs anyway
  with `--force`.
- **Replacing a binary was not atomic**: the old one was deleted and
  the new one written in its place, so an interrupted copy left no
  binary at all and an enabled unit restarting into nothing. It is
  written beside the target and renamed over it.
- **`podctl uninstall` could hand the wrong package to pacman.** It
  asked who owns `/usr/bin/podctl` while allowing itself to run from
  `/usr/local/bin`; it asks who owns the copy that is running.
- **`podctl uninstall` left the proximity keys and the settings
  behind** without mentioning them. Both are named in the listing, and
  `--purge` removes them.
- **The no-daemon marker was written where the CLI does not look for
  it** (always `~/.config`, never `XDG_CONFIG_HOME`), so the
  "running without daemon" banner kept appearing. All of podctl's paths
  come from one place now, and `--no-daemon` writes the marker as
  declining the question does.
- The zsh completion went to a directory that is not in a default
  `fpath`, under the old name `_pods`, with no hint. It is `_podctl`,
  the installer prints the `fpath` line when it is needed, and
  `uninstall` still removes the old file.
- `podctl install` claimed to have installed man pages even when
  writing them failed.
- **Every failure from the daemon exited 69**, so a script could not
  tell "no daemon, try again later" from "this model will never do
  that". Errors carry a kind now — `unavailable`, `unsupported`,
  `not_implemented`, `backend`, `internal`, `usage` — which the shell
  sees as 69, 1, 1, 71, 70 and 64, `--json` carries as `data.code`, and
  the man page spells out, including the fact that `status` exits 0 even
  when the AirPods are disconnected. The socket protocol is at 2 for
  it.
- **The daemon could be held open by a client that said nothing.** A
  connection now has five seconds to send its request, a request may be
  64 KB (an endless line used to buffer until the daemon was killed),
  and sixty-four connections are served at once — the sixty-fifth is
  turned away with a message instead of adding to the pile.
- **A `watch` client that went away left its task behind**, and with it
  its registration: `podctl popup` kept reporting success although
  nobody was there to draw the bubble. A hangup ends the stream now.
- **The daemon could cut its own connections.** It kept the number of
  the socket to the buds rather than the socket, and the kernel hands a
  closed number to whatever opens next — a `watch` stream or a BLE scan
  then ended for no visible reason.
- **Audio went to the first Bluetooth device, not to the AirPods.** With
  a speaker or headset also paired, volume, mute, profile, codec,
  default sink, latency, `podctl meter` and the ducking could all land
  on that one, and a bud coming out paused whatever was playing there.
- **`podctl profile off` was a dead end.** The card was looked up
  through its sink, and that profile has none, so profile, codec and
  latency answered "no AirPods card found" from then on.
- **A volume lowered for speech could stay down** after a dropped link,
  SIGTERM or a crash. It is noted on disk while it is lowered and
  restored on start and on reconnect, a watchdog covers a missing
  end-of-speech frame, and a volume you move yourself stays put.
- **Nothing on the way to the buds had a timeout**: a connect to buds
  out of range held the daemon's poll loop for the kernel's own, reads
  blocked the same way, and a write went out holding the lock on the
  socket.
- **The socket to the buds was inherited** by every process the daemon
  started.
- **The identity resolving key of the buds was written to the debug
  log.**
- **Device names reached the terminal unfiltered**, escape sequences and
  bidi overrides included.
- **`podctl pair` paired and trusted the first unpaired device that
  looked like AirPods.** It lists what it found and pairs what you name,
  or the only candidate with `--yes`.
- **`--help` ran the command** for `debug`, `meter`, `completion` and
  `version`.
- **`podctl ping` answered "pong" without a daemon**, and exited 0.
- **`podctl meter --json` never printed JSON**, and `--interval 0` spun
  a core.
- **The tray icon did not come back after a panel restart**, and a
  missing tray host made it exit into a restart loop.
- **The bubble could paint into a buffer the compositor was still
  reading**, and on a surface nobody reads from it could no longer be
  hidden or quit.
- **The notification fallback showed an empty notification.** It carries
  the device name and the levels as text now.
- A BLE battery reading above ten tenths was reported as up to 140 %.
- **Ear detection and the microphone mode were shown as nothing at all**
  until podctl had set them in that session: the Pro 2 USB-C leave both
  out of the settings they report. Both live in
  `~/.config/podctl/daemon.toml` now, so the switch that decides whether
  a bud coming out pauses playback survives a restart. Buds that do
  report them still have the last word.
- **`podctl pair` always failed with "Resource temporarily unavailable
  (os error 11)".** It answers only after a twelve-second scan, and the
  CLI gave up after ten — pairing through a running daemon could not
  work at all. Scanning and connecting get the time they need now, the
  CLI says it is scanning, and a wait that does run out says so instead
  of printing the raw error.
- **The notification fallback never appeared as a banner on GNOME.** It
  was sent as low urgency, which GNOME delivers straight into the
  message tray without showing it. It no longer sends the rendered card
  along either: a notification server lays that out as it likes, and
  GNOME squeezed the wide card into a small cropped square. The
  fallback is the device name, the levels as text and the headphones
  icon.
- **`podctl status` named the wrong codec.** The plain `a2dp-sink`
  profile was read as AAC, and a PipeWire built without AAC runs SBC in
  it — the sink itself is asked now. `podctl codec` lists what is really
  on offer, instead of offering an AAC that isn't there.
- **A missing `pactl` was reported as missing AirPods.** On Debian and
  Ubuntu `pipewire-pulse` does not bring `pactl` along, which the
  install notes got wrong too.
- `podctl meter` opened with one implausible reading — the first samples
  out of the capture are whatever its buffer held.
- `podctld.service` ordered itself against `bluetooth.service`, which a
  user unit cannot do, and messages pointed at `podctl setup`, which has
  never existed.

### Changed
- **The case-open bubble is redrawn.** Each model is drawn as it looks,
  buds and case on one millimetre scale (AirPods Max on its own), the
  buds level with the middle of the case, and the finger notch on every
  Pro case. The title is the name you gave the
  buds, cut with an ellipsis instead of shrunk; the line under it is the
  listening mode or "Connecting…". Both buds share one value when they
  are within 5 points; red means at or below 20 %, green charging, and
  there is no red frame any more. Text is drawn from the font's outlines,
  so `fontdue` is gone. `podctl-popup --dump` takes `--model`, `--scale`
  and `--backdrop`, `--demo` takes `--hold`, and a test compares all 28 renders with the
  reference images in `tests/golden`.
- **The bubble follows the desktop's light or dark style.** `theme` in
  `popup.toml` takes `auto`, now the default, besides `dark` and
  `light`. It reads the setting from the desktop portal (KDE, GNOME and
  others) and picks up a change from the next bubble on. Without a
  portal it stays dark, as before.
- **The bubble is sharp on scaled screens.** On Wayland it draws at the
  scale of the output it lands on, fractional ones such as 1.25 or 1.75
  included (`fractional-scale-v1` with `viewporter`, else the integer
  buffer scale); on X11 at `Xft.dpi` / 96. It used to draw at 1× and
  leave the stretching to the compositor.
- **The bubble answers to the pointer.** A click on the card closes it;
  while the pointer rests on it the bubble stays, and once it leaves at
  least 1.5 s remain. Over the close button it lights up and the pointer
  turns into a hand (`cursor-shape-v1` on Wayland, the cursor font's
  hand on X11). Only the card takes clicks, the shadow around it lets
  them through; on X11 the whole bubble used to be click-through.
- **The bubble moves in time, not in frames.** It rises 20 px into place
  on a spring while it fades in, fades out 10 px downwards after the hold,
  and a click shrinks it a little as it fades. The window stays put and
  the card moves inside it, paced by the compositor's frame callbacks on
  Wayland. Where nothing can blend (the notification fallback, X without
  a compositor) the card simply appears. `anim_ms` in `popup.toml` is no
  longer read.
- As the card rises the buds hop once, the left one first, and a
  charging case's light comes on. The products are drawn once into
  layers on a thread of their own and only put together per frame; once
  the entry is over the card is the one drawn in one go again.
- The levels come in instead of standing there: each ring fills in
  turn, the "—" makes way for the number as it rises into place, a
  charging case's bolt comes in after its ring in four short steps, and
  a low level pulses once. Values that arrive while "Connecting…" shows
  play the same way, and the status line changes without two texts ever
  overlapping. The Wayland bubble keeps three buffers, so a frame no
  longer waits a quarter of a second for one to come back.
- Changes while the bubble is up play out too: a ring slides to its new
  level as the old number leaves upwards and the new one comes in from
  below, a case put on charge turns green and lights up, and the shared
  reading for both buds splits into left and right when they drift
  apart, and back again. `--demo --updates` shows all of it.
- **Less motion when the desktop asks for it.** `motion` in `popup.toml`
  (`auto`, the default, `full`, `reduced`) follows GNOME's animations
  switch, KDE's animation speed and the portal's reduced-motion key; the
  bubble then only fades in and out, and its values just appear.
- The close button lights up and back down over 120 ms, and a click holds
  it down for a moment before the bubble shrinks away.
- `output` in `popup.toml` works on Wayland too. On X11 the bubble now
  goes to the monitor under the pointer before the primary one.
- The bubble draws about five times faster: shadows are blurred only
  where they fall, not across the whole buffer. A card at 2× took 1.7 s
  and now takes 0.3 s.
- The duck for speech starts as soon as the AirPods report it, by the
  smallest step, while the music's level is measured; it used to wait
  for the measurement first. About 0.7 s earlier, down after 1.3 s.
- **`podctl status` was rewritten.** One label column, labels that are
  the command names (`ear`, `loud-reduction`, `auto-connect`), one line
  per bud with a bar on a terminal and nothing but columns in a pipe,
  and the charging state as a word — the lightning bolt was two cells
  wide in most terminals, so the columns moved as soon as a bud
  charged. `trusted` is gone: it is the same BlueZ flag as
  `auto-connect` and read as two settings. Disconnected, `status`
  showed the word "disconnected" and nothing else; it now shows the
  levels the case last announced over bluetooth LE, with their age, and
  with nothing paired it says that instead. `podctl battery` prints the
  same rows, and `podctl list` cuts a long name to its column instead
  of shifting every line after it.
- `PODS_NO_LR` is now `PODCTL_NO_LR` (the old name still works) and is
  documented.
- **`--json` has one shape for every command**:
  `{"schema":1,"ok":true,"kind":"state","data":{…}}`, one object on one
  line, so a stream of them (`watch`, `meter`) is newline-delimited
  JSON. There were three shapes before — `{"kind":…,"data":…}` for most,
  `{"daemon":…}` for `version`, `{"ble":true}` for `ble` — some printed
  over forty lines, and `debug --json` ignored the flag. `--pretty`
  indents it for reading. The shape is in the man page under
  JSON OUTPUT and held by tests.
- **JSON spells values the way the command line takes them**, so
  `podctl mode "$(podctl status --json | jq -r .data.settings.mode)"`
  works. It used to print `noise_cancellation`, `high_quality` and
  `always_left`, none of which the CLI accepts. The old spellings are
  still read back.
- An error in JSON mode is an envelope with `ok: false`, the message and
  the hint, on standard output.
- `pending` answers print `pending: <reason>` instead of `ok (pending:
  …)` — it has not happened yet.
- The generated bash completion no longer needs the bash-completion
  package loaded; without it, it reads `COMP_WORDS` itself.
- **`podctl` and `podctld` have to be the same version** for the
  settings commands: the socket carries the same spellings, so a 0.3
  daemon answers a 0.4 `podctl mode anc` with "malformed request". The
  handshake above now says so in as many words; run `podctl reboot`
  after an upgrade.
- Connecting to the daemon, writing a line and reading one back was
  spelled out in nine places — three in the CLI, two each in the tray,
  the bubble and the debug report — every one with its own timeouts and
  its own idea of what an error is. One client in the library does it
  now, blocking or async, single answer or event stream.
- **The command line comes from one table now** (`src/commands.rs`).
  Help, the completion scripts and the argument checks were six
  hand-kept copies of the same list, and they had drifted: `podctl m
  <TAB>`, `podctl tone <TAB>` and `podctl --json mode <TAB>` completed
  nothing, the internal `debug emit-case-lid` was offered to everyone,
  and the zsh file installed itself as `_pods`. The overview has fixed
  columns and an alias column, `podctl completion <shell>` writes the
  script out from the table, and a test holds the files under `dist/`,
  the man page and the parser to it.
- **Surplus arguments are a usage error**, so `podctl rename Test
  AirPods` says to quote the name instead of renaming to "Test". An
  option a verb does not take is refused as well.
- **A mistyped command names the one you meant** ("did you mean
  'status'?"), and `podctl help nope` exits 64 instead of 0.
- **One shape for value errors**: `podctl: mode: unknown value 'foo'
  (expected off, anc, transparency, adaptive)`, with `hint:` on the
  next line. There were three hint styles and two quoting styles.
- `podctl meter --interval abc` fell back to 100 ms without a word and
  `--interval 0` was a busy loop; both are usage errors now.
- The four settings whose AAP ids are unknown (`spatial`,
  `loud-reduction`, `press`, `tone-on-press`) are marked as not
  implemented in the overview, their own help pages and the man page.
- The man page lists the aliases, documents `PODS_NO_LR`, and says that
  `uninstall` on a system install can run `sudo pacman -R`.
- **Release binaries are stripped**: 79 MB down to 13.5 MB for all four
  together. `thiserror` and tokio's `process` feature are gone; nothing
  used them.
- The bubble stands still instead of pushing sixty identical frames a
  second, and on X11 it re-uploads the picture only when it changed.
- Documentation: loud-sound reduction lists the models that have it, the
  four unimplemented settings are named as errors rather than no-ops,
  AirPods 4 non-ANC has a known product code, and the prebuilt tarball
  is mentioned where people look for an install.
- **The bubble shows on GNOME as itself.** GNOME lets no program float
  a window, so it used to fall back to XWayland or a plain notification.
  A small shell extension for GNOME 46–51 (`contrib/gnome-shell-extension/`)
  now puts the frames podctl-popup draws on screen, at the monitor's
  scale, with hover and click, and holds back GNOME's volume OSD while
  the bubble is up instead of letting it cover the card. `podctl install`
  offers it on GNOME, `podctl debug` says whether it runs, and
  `backend = "gnome"` picks it by name.
- **The tray icon is its own.** Both buds over a level bar for the lower
  one, red at 20 % and below, a bolt while they charge, faded while they
  are not connected — the theme's headphones looked the same either way,
  and a low level swapped them for the laptop's battery glyph. The icons
  are written to the tray's runtime directory at start, so nothing new
  has to be installed. `podctl-tray --demo` shows them all.
- **The tray menu is rebuilt.** Name and model on top, a line per bud
  and the case that is no longer greyed out, the modes in the order of
  an iPhone, and submenus for volume, audio output and the microphone.
  What the model cannot do is hidden instead of greyed out; not connected
  it offers Connect first and the last levels the case announced.
- The low-battery warning is per bud and comes in two steps (20 % and
  10 %, critical), names the bud ("Left AirPod at 9% · Right 34%"),
  replaces the previous one instead of piling up, and does not come back
  when the other bud goes into the case for a moment.
- The tray's tooltip carries the name you gave the buds, the levels and
  the mode, and says "Not connected" when they are not.
- The tray unit gets its own runtime directory and a tighter sandbox
  (`ProtectHome=read-only`, `MemoryDenyWriteExecute`, Unix sockets only).

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
