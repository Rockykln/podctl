//! User-facing help text. All wording lives here so it stays consistent
//! and adding/renaming a verb is a single edit.

pub fn print_top(focus: Option<&str>) {
    if let Some(v) = focus {
        print_verb(v);
        return;
    }
    println!("{TOP_HELP}");
}

pub fn print_verb(verb: &str) {
    let text = match verb {
        "status" | "s" => STATUS,
        "battery" | "bat" | "b" => BATTERY,
        "mode" | "m" => MODE,
        "conv" | "c" => CONV,
        "spatial" => SPATIAL,
        "ear" | "ear-detection" => EAR,
        "mic" => MIC,
        "loud-reduction" | "loud" => LOUD,
        "press" => PRESS,
        "tone-on-press" | "tone" => TONE,
        "rename" => RENAME,
        "connect" => CONNECT,
        "disconnect" | "dc" => DISCONNECT,
        "pair" => PAIR,
        "unpair" | "forget" => UNPAIR,
        "list" | "ls" => LIST,
        "auto-connect" | "auto" => AUTO,
        "volume" | "vol" | "v" => VOLUME,
        "mute" => MUTE,
        "profile" | "p" => PROFILE,
        "codec" => CODEC,
        "default" | "default-sink" => DEFAULT_SINK,
        "latency" => LATENCY,
        "watch" | "w" => WATCH,
        "meter" => METER,
        "one-bud-anc" | "obanc" => ONE_BUD_ANC,
        "chime" | "chime-volume" => CHIME,
        "auto-anc" | "anc-strength" => AUTO_ANC,
        "completion" => COMPLETION,
        "ping" => PING,
        "debug" => DEBUG,
        "tray" => TRAY,
        "popup" => POPUP,
        "reboot" => REBOOT,
        "install" => INSTALL,
        "uninstall" => UNINSTALL,
        "version" | "--version" | "-V" => VERSION,
        "help" | "-h" | "--help" => TOP_HELP,
        other => {
            eprintln!("no help for '{other}' — try 'podctl help' for the command list.");
            return;
        }
    };
    println!("{text}");
}

const TOP_HELP: &str = "\
podctl — control AirPods from the Linux terminal

USAGE
    podctl <command> [args]
    podctl help [command]

CORE
    status      (s)             full snapshot — everything we know
    battery     (b|bat)         battery for left, right and case
    ping                        daemon health check

LISTENING
    mode        (m)   <off|anc|transparency|adaptive>
    conv        (c)   <on|off>      conversation awareness
    spatial           <off|fixed|head-tracked>

BUD SETTINGS
    ear               <on|off>      in-ear auto-pause detection
    mic               <auto|left|right>
    loud-reduction    <on|off>      cap loud spikes
    press             <left|right> <mode-cycle|siri|none>
    tone              <on|off>      acoustic feedback on press
    rename            \"name\"        change the bluetooth name
    one-bud-anc       <on|off>      keep ANC active with only one bud in
    auto-anc          <0..100>      AutoANC strength on Pro 2
    chime             <0..100>      system tone / chime volume

BLUETOOTH  (BlueZ)
    connect                     bring the BT link up
    disconnect  (dc)            drop the BT link
    pair                        pair the AirPods near the dongle
    unpair      (forget)        remove the pairing
    list        (ls)            all paired AirPods on the system
    auto        <on|off>        BlueZ Trusted flag

AUDIO  (PipeWire / PulseAudio)
    volume      (v|vol)   <0..100>
    mute        <on|off>
    profile     (p)   <high|headset|off>
    codec             <sbc|aac|aptx|ldac|…>
    default                     set AirPods as default audio sink
    latency           <ms>      output latency offset (can be negative)

STREAMING
    watch       (w)             live event stream (mode, in-ear, presses…)
    meter       [--plain|--json] [--interval N] [--device <sink-monitor>]
                                live RMS / peak dBFS of what you're sending
                                to the AirPods (software meter, not SPL)

UI
    tray        <start|stop|status|restart>   status-bar icon
    popup                       show the case-open bubble now
    reboot                      restart podctld + tray + popup

MISC
    completion  <bash|zsh|fish> shell completion script to stdout
    debug       [--no-redact]   paste-friendly diagnostic report
    install     [--yes]         install to ~/.local (binaries + completion + man + service)
    uninstall   [--yes]         remove everything 'install' put in place
    help        [cmd]
    version                     CLI + daemon version

EXAMPLES
    podctl status                 # full snapshot, AAP-live
    podctl m anc                  # noise cancellation
    podctl auto-anc 60            # adaptive ANC at 60 %
    podctl watch                  # streams events while you wear them
    podctl meter                  # live dB meter on the playback sink
    podctl v 60                   # volume 60 %

The daemon `podctld` must be running:
    systemctl --user start podctld
";

const STATUS: &str = "\
podctl status — full snapshot

Prints everything the daemon knows: model, firmware, battery, in-ear
state, listening mode, conversation awareness, bud settings, audio
state (volume/profile/codec), bluetooth state (RSSI/trusted/auto-connect),
press counts.

The 'updated' field shows how fresh the data is.
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

Lowers media volume + weakens ANC when your own voice is detected.
Only AirPods Pro 2 and AirPods 4 ANC.

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

    podctl spatial fixed
";

const EAR: &str = "\
podctl ear — in-ear auto-pause

When on, the bud reports being worn / removed; audio pauses on removal.
Off freezes the bud's behaviour: audio plays even when both buds are out.

    podctl ear on
    podctl ear off
";

const MIC: &str = "\
podctl mic — microphone selection

    auto    bud picks based on noise + in-ear state
    left    always use left bud's mic
    right   always use right bud's mic

    podctl mic auto
";

const LOUD: &str = "\
podctl loud-reduction — cap loud sound spikes

Apple's hearing-safety filter. Pro 2 / AirPods 4 ANC / Max.

    podctl loud-reduction on
    podctl loud off
";

const PRESS: &str = "\
podctl press — set stem press-and-hold action

    podctl press <left|right> <mode-cycle|siri|none>

actions:
    mode-cycle   cycle through ANC → Transparency → Off (recommended on Linux)
    siri         trigger Siri (no-op on Linux — bud sends event into the void)
    none         do nothing on long-press

    podctl press left  mode-cycle
    podctl press right siri
";

const TONE: &str = "\
podctl tone — acoustic feedback on press

Plays a small 'click' through the bud when a stem press registers.
Off makes presses silent.

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
LED blinks white), then run this. The daemon will scan, find them, and
pair via BlueZ.

    podctl pair
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
podctl auto — auto-connect on adapter power-up

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
    - in-ear / out-of-ear
    - case lid open / close
    - mode / conversation-awareness changes
    - stem press events (single / double / triple / long)

Good for shell automation:

    podctl watch | while read -r line; do
      case \"$line\" in
        *in_ear*false*) podctl mute on  ;;
        *in_ear*true*)  podctl mute off ;;
      esac
    done

    podctl watch
    podctl w
";

const METER: &str = "\
podctl meter — live software dB meter on the AirPods playback sink

Reads the bluez_output.<MAC>.monitor PulseAudio / PipeWire source and
prints RMS + peak in dBFS for whatever audio is currently being sent
to the AirPods. This is NOT a real SPL/loudness meter — it measures
the digital signal, not what your eardrum actually receives.

OPTIONS
    --interval <ms>     update window length (default 100ms)
    --device <name>     PulseAudio source name (default: auto-detect)
    --plain             one line per update (no in-place TTY refresh)
    --json              JSON object per update for scripts
    --once              measure one window then exit

Requires `parec` from pulseaudio-utils.
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
    podctl completion zsh     > ~/.local/share/zsh/site-functions/_pods
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
right place for your shell (fish/bash/zsh autodetected from \\$SHELL),
installs man pages to ~/.local/share/man/man1/, and asks whether you
want a systemd user service that keeps the daemon running. When the
podctl-tray / podctl-popup binaries sit next to podctl it offers those
too — the tray's left-click needs podctl-popup to draw anything.

No root needed. Run from the build directory or anywhere the binaries
are co-located.

FLAGS
    --yes              non-interactive — assume yes to every prompt
    --no-daemon        skip the systemd unit
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
to remove it yourself.

    podctl uninstall
    podctl uninstall --yes
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

const PING: &str = "\
podctl ping — daemon health check

Round-trips one request to make sure the socket is live. Prints 'pong'.

    podctl ping
";

const TRAY: &str = "\
podctl tray — status-bar icon (podctl-tray)

Wraps the podctl-tray systemd user service. The icon shows connection
and battery (via its tooltip). Left-click shows the popup by default;
right-click opens the menu (mode, conversation awareness, disconnect).

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
