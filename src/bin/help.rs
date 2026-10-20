//! User-facing help text. The overview is built from the command table
//! (`podctl::commands`); the per-verb pages live here.

use podctl::commands::{self, Cmd, State};
use podctl::exitcode;

pub fn print_top(focus: Option<&str>) -> i32 {
    if let Some(v) = focus {
        return print_verb(v);
    }
    println!("{}", top_help());
    exitcode::OK
}

/// The overview: one line per command, grouped, in fixed columns. Only
/// the short aliases are listed; the longer alternate spellings
/// (`ear-detection`, `anc-strength`, …) work the same and are in the
/// man page.
fn top_help() -> String {
    let mut out = String::from(HEAD);
    for group in commands::GROUPS {
        out.push('\n');
        out.push_str(group);
        out.push('\n');
        for c in commands::COMMANDS.iter().filter(|c| &c.group == group) {
            let args = c.args_label();
            // One long argument list (`press`) must not push into its
            // own description.
            let gap = if args.len() >= ARGS_COL { " " } else { "" };
            // The four whose AAP ids are unknown say so instead of
            // describing what they would do; `podctl help <verb>` has
            // the detail.
            let summary = if c.state == State::NotImplemented {
                "(not implemented yet)"
            } else {
                c.summary
            };
            let line = format!(
                "  {:<16}{:<8}{:<width$}{gap}{summary}",
                c.name,
                short_aliases(c),
                args,
                width = ARGS_COL,
            );
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out.push('\n');
    out.push_str(TAIL);
    out
}

const ARGS_COL: usize = 32;

fn short_aliases(c: &Cmd) -> String {
    c.aliases
        .iter()
        .filter(|a| a.len() <= 6)
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

/// Help for one verb, by any of its names. Returns the exit code: an
/// unknown topic is a usage error, not a success.
pub fn print_verb(verb: &str) -> i32 {
    let canonical = commands::find(verb).map(|c| c.name).unwrap_or(verb);
    let text = match canonical {
        "status" => STATUS,
        "battery" => BATTERY,
        "mode" => MODE,
        "conv" => CONV,
        "spatial" => SPATIAL,
        "ear" => EAR,
        "mic" => MIC,
        "loud-reduction" => LOUD,
        "press" => PRESS,
        "tone-on-press" => TONE,
        "rename" => RENAME,
        "connect" => CONNECT,
        "disconnect" => DISCONNECT,
        "pair" => PAIR,
        "unpair" => UNPAIR,
        "list" => LIST,
        "auto-connect" => AUTO,
        "volume" => VOLUME,
        "mute" => MUTE,
        "profile" => PROFILE,
        "codec" => CODEC,
        "default" => DEFAULT_SINK,
        "latency" => LATENCY,
        "watch" => WATCH,
        "meter" => METER,
        "ble" => BLE,
        "one-bud-anc" => ONE_BUD_ANC,
        "chime" => CHIME,
        "auto-anc" => AUTO_ANC,
        "completion" => COMPLETION,
        "ping" => PING,
        "debug" => DEBUG,
        "tray" => TRAY,
        "popup" => POPUP,
        "reboot" => REBOOT,
        "install" => INSTALL,
        "uninstall" => UNINSTALL,
        "version" | "--version" | "-V" => VERSION,
        "help" | "-h" | "--help" => {
            println!("{}", top_help());
            return exitcode::OK;
        }
        other => {
            eprintln!("podctl: no help for '{other}'");
            if let Some(hint) = commands::suggest(other) {
                eprintln!("hint: did you mean '{hint}'? see 'podctl help'");
            } else {
                eprintln!("hint: 'podctl help' lists every command");
            }
            return exitcode::USAGE;
        }
    };
    println!("{text}");
    exitcode::OK
}

const HEAD: &str = "\
podctl — control AirPods from the Linux terminal

USAGE
    podctl <command> [args]
    podctl help [command]

OPTIONS
    --json          answer as one JSON object per line
    --pretty        with --json, indent it over several lines
";

const TAIL: &str = "\
EXAMPLES
    podctl status                 # everything we know, live from the buds
    podctl m anc                  # noise cancellation
    podctl auto-anc 60            # adaptive ANC at 60 %
    podctl watch                  # streams events while you wear them
    podctl meter                  # live dB meter on the playback sink
    podctl v 60                   # volume 60 %

Settings on the buds themselves (mode, conv, ear, …) need podctld:
    systemctl --user enable --now podctld
Audio, bluetooth and 'ble' also work without it.
";

const STATUS: &str = "\
podctl status — everything we know

    AirPods Pro (2nd gen, USB-C)  \"My AirPods\"  connected · AAP up · 0s ago

      battery   left    80%  [########--]  charging
                right   75%  [########--]
                case    40%  [####------]
      in-ear    both in ear   case lid closed

      listening mode anc · conv on
      settings  ear on · mic auto · one-bud-anc off · auto-anc 50 · chime 100
      audio     60%  high  aac  [default sink]
                codecs: sbc, aac
      bluetooth AA:BB:CC:DD:EE:01  rssi -52 dBm  auto-connect on
      firmware  L 7A305 · R 7A305 · case 7A305

The labels are the command names, so what you read is what you can
type. Bars appear on a terminal and not in a pipe. Disconnected, the
levels the case last announced over bluetooth LE are shown with their
age; with nothing paired, status says so.

The age after the state says how fresh the daemon's picture is.
It exits 0 either way — read 'connected' from --json to test for a
link.
";

const BATTERY: &str = "\
podctl battery — battery levels

Per-component levels with a charging indicator. Updates roughly every
second from the proprietary BLE advertisement.

    podctl battery
    podctl b
";

const MODE: &str = "\
podctl mode — switch listening mode

    off            passive
    anc            active noise cancellation
    transparency   pass outside sound through
    adaptive       Pro 2 / AirPods 4 ANC only

Aliases: nc/noise/noise-cancellation → anc, tr/trans/pass → transparency.

    podctl mode anc
    podctl m transparency
";

const CONV: &str = "\
podctl conv — conversation awareness

When your own voice is detected the buds switch towards transparency
and podctld fades the music down to a level you can talk over: it
measures how loud the music plays and lowers it by 6 to 30 dB to match,
fading back 1.5 s after you stop. Only AirPods Pro 2 and AirPods 4 ANC.

    podctl conv on
    podctl c off
";

const SPATIAL: &str = "\
podctl spatial — spatial audio mode

    off            mono/stereo as-is
    fixed          virtualised, no head tracking
    head-tracked   uses bud IMU, follows head rotation

Linux doesn't render personalised spatial audio (Apple-only); this only
sets the bud's stored preference.

Not implemented yet: the AAP setting id for this is not known, so
the daemon answers with an error instead of pretending.

    podctl spatial fixed
";

const EAR: &str = "\
podctl ear — in-ear auto-pause

When on, taking a bud out pauses every playing media player (MPRIS)
and putting it back resumes the ones podctld paused, as on an iPhone.
Buds that return after 15 minutes don't restart playback. Needs the
daemon; only acts while the AirPods are the default output.

    podctl ear on
    podctl ear off

The buds are told as well, but they don't report the switch back — the
Pro 2 USB-C leave it out of the settings dump entirely. podctl therefore
keeps it in ~/.config/podctl/daemon.toml ('ear_detection = true'), which
is also what decides whether it pauses. It starts out on.
";

const MIC: &str = "\
podctl mic — microphone selection

    auto    bud picks based on noise + in-ear state
    left    always use left bud's mic
    right   always use right bud's mic

    podctl mic auto

Like ear detection, the buds keep this to themselves, so podctl remembers
it in ~/.config/podctl/daemon.toml ('mic = auto').
";

const LOUD: &str = "\
podctl loud-reduction — cap loud sound spikes

Apple's hearing-safety filter. Pro 2 / AirPods 4 ANC / Max.

Not implemented yet: the AAP setting id for this is not known, so
the daemon answers with an error instead of pretending.

    podctl loud-reduction on
    podctl loud off
";

const PRESS: &str = "\
podctl press — set stem press-and-hold action

    podctl press <left|right> <mode-cycle|siri|none>

actions:
    mode-cycle   cycle through ANC → Transparency → Off
    siri         trigger Siri (no-op on Linux — bud sends event into the void)
    none         do nothing on long-press

Not implemented yet: the AAP setting id for this is not known, so
the daemon answers with an error instead of pretending.

    podctl press left  mode-cycle
    podctl press right siri
";

const TONE: &str = "\
podctl tone-on-press — acoustic feedback on press

Plays a small 'click' through the bud when a stem press registers.
Off makes presses silent.

Not implemented yet: the AAP setting id for this is not known, so
the daemon answers with an error instead of pretending.

    podctl tone on
    podctl tone off
";

const RENAME: &str = "\
podctl rename — change the bluetooth name

Renames the AirPods as they appear in BlueZ + the AAP-reported name.

    podctl rename \"Studio Podctl\"
";

const CONNECT: &str = "\
podctl connect — bring up the bluetooth link

Asks BlueZ to connect to the paired AirPods. Equivalent to
'bluetoothctl connect <MAC>'.

    podctl connect
";

const DISCONNECT: &str = "\
podctl disconnect — drop the bluetooth link

Pairing is preserved; 'podctl connect' brings it back.

    podctl disconnect
    podctl dc
";

const PAIR: &str = "\
podctl pair — pair a nearby AirPods

Put the AirPods in pairing mode (long-press the case button until the
LED blinks white), then run this. It scans for twelve seconds and shows
what it found; pairing then needs either the address or --yes, because
anything in pairing mode within a few metres shows up here, including
somebody else's.

    podctl pair                   # scan and list what is in range
    podctl pair --yes             # take the one candidate, if there is
                                  # exactly one
    podctl pair AA:BB:CC:DD:EE:FF # pair that one

    podctl status                 # afterwards: should show them
";

const UNPAIR: &str = "\
podctl unpair — forget the AirPods

Removes the BlueZ pairing. To use the AirPods again you'll need to pair
fresh.

    podctl unpair
    podctl forget
";

const LIST: &str = "\
podctl list — list paired AirPods

Shows every AirPods-family device the BlueZ adapter knows about, with
connected/trusted flags. Useful when you have multiple pairs.

    podctl list
    podctl ls
";

const AUTO: &str = "\
podctl auto-connect — connect on adapter power-up

Maps to BlueZ's 'Trusted' flag. When on, BlueZ tries to reconnect the
AirPods automatically when the adapter is enabled or they come back in
range.

    podctl auto on
    podctl auto off
";

const VOLUME: &str = "\
podctl volume — set audio sink volume (0..100)

Applies to the PipeWire sink for the AirPods. Same effect as
'wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.6' when AirPods are default.

    podctl volume 60
    podctl v 100
";

const MUTE: &str = "\
podctl mute — mute / unmute the AirPods sink

    podctl mute on
    podctl mute off
";

const PROFILE: &str = "\
podctl profile — bluetooth audio profile

    high      a2dp_sink            stereo, sink-only, best fidelity
    headset   headset_head_unit    HSP/HFP, mic + speaker, mono
    off       profile disabled

Linux runs only one of these at a time per BT card.

    podctl profile high
    podctl p headset
";

const CODEC: &str = "\
podctl codec — A2DP codec

Selects the codec the bluetooth audio stream uses. Available codecs come
from PipeWire — usually 'sbc' and 'aac'; some builds also expose
'aptx_hd' / 'ldac'. Run 'podctl status' for the list reported as
'available_codecs'.

    podctl codec aac
";

const DEFAULT_SINK: &str = "\
podctl default — set AirPods as default audio sink

Equivalent to 'wpctl set-default <airpods-sink-id>'.

    podctl default
";

const LATENCY: &str = "\
podctl latency — sink-side latency offset in ms

Negative values delay audio (sync to slow video); positive values play
earlier (sync to fast video).

    podctl latency 0
    podctl latency -50
";

const WATCH: &str = "\
podctl watch — live event stream

Long-lived subscription. Prints one event per line until you Ctrl-C:
    - connected / disconnected
    - battery changes
    - in-ear / out-of-ear / in-case
    - case lid open / close
    - mode / conversation-awareness changes

With --json every line is one event object, e.g.
    {\"kind\":\"event\",\"data\":{\"event\":\"mode\",\"mode\":\"transparency\"}}

Good for shell automation:

    podctl watch --json | while read -r line; do
      case \"$line\" in
        *'\"event\":\"mode\"'*) notify-send AirPods \"$line\" ;;
      esac
    done

    podctl watch
    podctl w
";

const METER: &str = "\
podctl meter — live software dB meter on the AirPods playback sink

Captures the bluez_output.<MAC> sink and prints RMS + peak in dBFS for
whatever audio is currently being sent to the AirPods. This is NOT a
real SPL/loudness meter — it measures the digital signal, not what your
eardrum actually receives.

OPTIONS
    --interval <ms>     update window length (default 100ms)
    --device <name>     sink name (default: the AirPods sink)
    --plain             one line per update (no in-place TTY refresh)
    --json              JSON object per update for scripts
    --once              measure one window then exit

Requires `pw-record` (pipewire-audio); falls back to `parec` from
pulseaudio-utils, which reads the sink monitor and stays silent for
Bluetooth sinks on PipeWire.
";

const ONE_BUD_ANC: &str = "\
podctl one-bud-anc — keep ANC active when only one bud is worn

    podctl one-bud-anc on
    podctl one-bud-anc off

Without this the AirPods normally drop out of ANC when only one bud
is in the ear, so transparency-by-default kicks in. Pro 2 only.
";

const CHIME: &str = "\
podctl chime — system tone / chime volume on the buds

    podctl chime 50      # quieter
    podctl chime 100     # loudest (default)
    podctl chime 0       # silent

Affects the small acoustic feedback you hear on mode switches and
case open/close. Range 0..100. Note: AirPods firmware may quantise.
";

const AUTO_ANC: &str = "\
podctl auto-anc — AutoANC / Adaptive Audio strength

    podctl auto-anc 0        # let the buds decide
    podctl auto-anc 50
    podctl auto-anc 100

Sets how aggressively the Pro 2 mixes ANC with transparency in
'Adaptive' mode. Range 0..100.
";

const COMPLETION: &str = "\
podctl completion — emit shell completion script

    podctl completion bash    > ~/.local/share/bash-completion/completions/podctl
    podctl completion zsh     > ~/.local/share/zsh/site-functions/_podctl
    podctl completion fish    > ~/.config/fish/completions/podctl.fish

Each script is self-contained — no runtime dependency on podctl being
in $PATH at completion-build time.
";

const VERSION: &str = "\
podctl version — print CLI version + daemon status

Same as 'podctl --version' and 'podctl -V'. Reports the binary version, the
license, and whether the daemon is reachable on the local socket. Exit
code is always 0; use 'podctl ping' if you need a hard health check.

    podctl version
    podctl --version
    podctl -V
";

const INSTALL: &str = "\
podctl install — install podctl globally for your user

Copies the binaries to ~/.local/bin/, drops shell completion into the
right place for your shell (fish/bash/zsh autodetected from $SHELL),
installs man pages to ~/.local/share/man/man1/, and asks whether you
want a systemd user service that keeps the daemon running. When the
podctl-tray / podctl-popup binaries sit next to podctl it offers those
too — the tray's left-click needs podctl-popup to draw anything.

No root needed. Run from the build directory or anywhere the binaries
are co-located. Services that are already running are restarted, so an
upgrade takes effect instead of leaving the old copy running.

If a distro package (podctl-bin, podctl-git) already provides podctl,
install stops: ~/.local/bin would shadow it in $PATH and the units in
~/.config would take precedence, so every later package upgrade would
have no effect. Either use the packaged services —
'systemctl --user enable --now podctld' — or pass --force.

FLAGS
    --yes              non-interactive — assume yes to every prompt
    --force            install even though a distro package provides podctl
    --no-daemon        skip the systemd unit (writes the no-daemon marker)
    --no-completion    skip shell completion
    --no-manpages      skip man pages
    --no-tray          do not offer the tray icon
    --no-popup         do not offer the case-open bubble
    --with-tray        install the tray icon without asking
    --with-popup       install the case-open bubble without asking

EXAMPLES
    podctl install
    podctl install --yes --no-daemon
    podctl install --with-tray --with-popup

After install: re-open your shell so completion + PATH kick in.
";

const UNINSTALL: &str = "\
podctl uninstall — remove what 'install' put in place

Removes binaries from ~/.local/bin/, the systemd user unit (after
disabling it), the completion script, the man pages, and the no-daemon
marker. Idempotent: missing files are ignored.

Only ever touches what it lists. If podctl is *also* installed as a
distro package and you run this from ~/.local/bin, the package and its
services are left running — the listing names it and the pacman command
to remove it yourself. Which package is asked about is the one that owns
the copy you are running, not whatever owns /usr/bin/podctl.

Your settings (~/.config/podctl) and the proximity keys
(~/.local/state/podctl, which is what lets podctl recognise your buds'
rotating bluetooth address) are left in place and named in the listing.
--purge removes them too.

Run from a system install, the listing instead includes the package
itself, and confirming it (or passing --yes) runs
'sudo pacman -R --noconfirm <package>'. Read the listing before you
agree to it.

    podctl uninstall
    podctl uninstall --yes
    podctl uninstall --purge     # settings and keys as well
";

const DEBUG: &str = "\
podctl debug — diagnostic report

Prints a structured report suitable for pasting into a bug report or
chat: versions, tools, daemon state, AirPods model + capabilities,
PipeWire sink/card/profile, audio codec list.

By default, identifying data is redacted: MAC addresses show only the
Apple OUI (first three bytes), custom Bluetooth names become
'<custom name redacted>', and home/runtime paths are normalised.

    podctl debug                  # safe to paste in public
    podctl debug --no-redact      # full info, LOCAL ONLY

EXAMPLES

    podctl debug | pbcopy          # macOS clipboard
    podctl debug | wl-copy         # Wayland clipboard
    podctl debug > report.txt
";

const BLE: &str = "\
podctl ble — listening for the case over Bluetooth LE

While the buds are disconnected the daemon can listen for the case's own
Bluetooth LE announcement. That is what makes the bubble appear as the
lid opens, three to five seconds before the classic link is back, and it
carries the battery levels shown in it.

Only announcements that resolve against the key the buds handed out over
the connected link are read; everything else another Apple device nearby
sends is dropped. The key stays in ~/.local/state/podctl/, mode 0600.

    podctl ble                    # show the current setting
    podctl ble off                # stop listening
    podctl ble on                 # listen again

The setting lives in ~/.config/podctl/daemon.toml and can be edited
there too, next to 'ear_detection' and 'mic'. A running daemon picks up
a change within a few seconds; no restart needed.
";

const PING: &str = "\
podctl ping — daemon health check

Round-trips one request to make sure the socket is live. Prints 'pong'.

    podctl ping
";

const TRAY: &str = "\
podctl tray — status-bar icon (podctl-tray)

Wraps the podctl-tray systemd user service. The icon shows connection
and battery (via its tooltip). Right-click opens the menu: battery,
the four listening modes, conversation awareness, connect/disconnect.

Mouse on the icon:

    left click      show the bubble (configurable, see below)
    double click    toggle ANC <-> Transparency
    middle click    toggle ANC <-> Transparency
    wheel           volume, in steps of five

Left-click action is configurable in ~/.config/podctl/tray.toml:

    left_click = popup          # show the bubble (default)
    left_click = mode-cycle     # cycle Off/ANC/Transparency/Adaptive
    left_click = toggle-anc-tr  # toggle ANC <-> Transparency
    left_click = menu           # do nothing (use right-click menu)

    podctl tray start
    podctl tray stop
    podctl tray status           # is it running? is a tray host present?
    podctl tray restart
";

const POPUP: &str = "\
podctl popup — show the case-open bubble now

Asks the running podctl-popup to slide in the current snapshot (model,
connection, battery, listening mode), the same bubble shown when the
case opens, the mode changes, or the AirPods connect. Needs podctld and a
running podctl-popup.

    podctl popup
";

const REBOOT: &str = "\
podctl reboot — restart podctl

Restarts the installed systemd user services (podctld, then podctl-tray and
podctl-popup if present) so a fresh build or a wedged daemon comes back
cleanly. No root required. Units that are not installed are skipped.

    podctl reboot
";
