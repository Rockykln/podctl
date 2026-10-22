//! The command table: one entry per verb, and the single source for the
//! parser, the help page, the completion scripts and the man-page test.

/// Whether the verb does anything on a device today. The four settings
/// whose AAP ids are not known are advertised as unsupported rather
/// than failing after the fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Ready,
    NotImplemented,
}

/// One positional argument. `values` are the choices shown in the help
/// and offered by the completions; empty means free text.
#[derive(Clone, Copy, Debug)]
pub struct Arg {
    pub label: &'static str,
    pub values: &'static [&'static str],
    pub required: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Cmd {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub group: &'static str,
    pub args: &'static [Arg],
    /// Shown after the arguments, for verbs whose options matter more
    /// than their arguments. Not counted when checking how many
    /// arguments were passed.
    pub hint: &'static str,
    pub flags: &'static [&'static str],
    pub summary: &'static str,
    pub state: State,
}

impl Cmd {
    /// How the arguments read in the help: `<on|off>`, `[on|off]`. A
    /// label that already carries its own brackets or quotes is left
    /// alone.
    pub fn args_label(&self) -> String {
        let mut parts: Vec<String> = self
            .args
            .iter()
            .map(|a| {
                if a.label.starts_with(['<', '[', '"', '-']) {
                    a.label.to_string()
                } else if a.required {
                    format!("<{}>", a.label)
                } else {
                    format!("[{}]", a.label)
                }
            })
            .collect();
        if !self.hint.is_empty() {
            parts.push(self.hint.to_string());
        }
        parts.join(" ")
    }

    /// How many positional arguments the verb takes at most.
    pub fn max_args(&self) -> usize {
        self.args.len()
    }

    /// How many it needs.
    pub fn min_args(&self) -> usize {
        self.args.iter().filter(|a| a.required).count()
    }

    pub fn matches(&self, word: &str) -> bool {
        self.name == word || self.aliases.contains(&word)
    }

    /// Choices for the n-th positional argument, for the completions.
    pub fn values_at(&self, index: usize) -> &'static [&'static str] {
        self.args.get(index).map(|a| a.values).unwrap_or(&[])
    }
}

const fn req(label: &'static str, values: &'static [&'static str]) -> Arg {
    Arg {
        label,
        values,
        required: true,
    }
}

const fn opt(label: &'static str, values: &'static [&'static str]) -> Arg {
    Arg {
        label,
        values,
        required: false,
    }
}

const ON_OFF: &[&str] = &["on", "off"];

pub const CORE: &str = "CORE";
pub const LISTENING: &str = "LISTENING";
pub const BUDS: &str = "BUD SETTINGS";
pub const BLUETOOTH: &str = "BLUETOOTH  (BlueZ)";
pub const AUDIO: &str = "AUDIO  (PipeWire / PulseAudio)";
pub const STREAMING: &str = "STREAMING";
pub const UI: &str = "UI";
pub const MISC: &str = "MISC";

/// Groups in the order the help prints them.
pub const GROUPS: &[&str] = &[CORE, LISTENING, BUDS, BLUETOOTH, AUDIO, STREAMING, UI, MISC];

macro_rules! cmd {
    ($name:expr, $aliases:expr, $group:expr, $args:expr, $flags:expr, $summary:expr, $state:expr) => {
        cmd!($name, $aliases, $group, $args, "", $flags, $summary, $state)
    };
    ($name:expr, $aliases:expr, $group:expr, $args:expr, $hint:expr, $flags:expr, $summary:expr, $state:expr) => {
        Cmd {
            name: $name,
            aliases: $aliases,
            group: $group,
            args: $args,
            hint: $hint,
            flags: $flags,
            summary: $summary,
            state: $state,
        }
    };
}

use State::{NotImplemented, Ready};

// Read as a table, so it stays one.
#[rustfmt::skip]
pub const COMMANDS: &[Cmd] = &[
    cmd!("status", &["s"], CORE, &[], &[], "everything we know right now", Ready),
    cmd!("battery", &["b", "bat"], CORE, &[], &[], "left, right and case", Ready),
    cmd!("ping", &[], CORE, &[], &[], "daemon health check", Ready),

    cmd!("mode", &["m"], LISTENING,
        &[req("off|anc|transparency|adaptive|next",
              &["off", "anc", "transparency", "adaptive", "next"])],
        &[], "listening mode", Ready),
    cmd!("conv", &["c"], LISTENING, &[req("on|off", ON_OFF)], &[],
        "conversation awareness", Ready),
    cmd!("spatial", &[], LISTENING,
        &[req("off|fixed|head-tracked", &["off", "fixed", "head-tracked"])], &[],
        "spatial audio", NotImplemented),

    cmd!("ear", &["ear-detection"], BUDS, &[req("on|off", ON_OFF)], &[],
        "pause when a bud comes out", Ready),
    cmd!("mic", &[], BUDS, &[req("auto|left|right", &["auto", "left", "right"])], &[],
        "which bud's microphone", Ready),
    cmd!("loud-reduction", &["loud"], BUDS, &[req("on|off", ON_OFF)], &[],
        "cap loud spikes", NotImplemented),
    // Values in `podctl help press`: spelling them out here is the one
    // line that would not fit the column.
    cmd!("press", &[], BUDS,
        &[req("left|right", &["left", "right"]),
          req("action", &["mode-cycle", "siri", "none"])], &[],
        "press-and-hold action", NotImplemented),
    cmd!("tone-on-press", &["tone"], BUDS, &[req("on|off", ON_OFF)], &[],
        "sound on press", NotImplemented),
    cmd!("rename", &[], BUDS, &[req("\"name\"", &[])], &[],
        "change the bluetooth name", Ready),
    cmd!("one-bud-anc", &["obanc"], BUDS, &[req("on|off", ON_OFF)], &[],
        "keep ANC with one bud in", Ready),
    cmd!("auto-anc", &["anc-strength"], BUDS, &[req("0..100", &[])], &[],
        "AutoANC strength (Pro 2)", Ready),
    cmd!("chime", &["chime-volume"], BUDS, &[req("0..100", &[])], &[],
        "volume of the bud's tones", Ready),

    cmd!("connect", &[], BLUETOOTH, &[], &[], "bring the link up", Ready),
    cmd!("disconnect", &["dc"], BLUETOOTH, &[], &[], "drop the link", Ready),
    cmd!("pair", &[], BLUETOOTH, &[opt("address", &[])], "[--yes]", &["--yes", "-y"],
        "pair AirPods in pairing mode", Ready),
    cmd!("unpair", &["forget"], BLUETOOTH, &[], &[], "remove the pairing", Ready),
    cmd!("list", &["ls"], BLUETOOTH, &[], &[], "every paired AirPods", Ready),
    cmd!("auto-connect", &["auto"], BLUETOOTH, &[req("on|off", ON_OFF)], &[],
        "BlueZ trusted flag", Ready),

    cmd!("volume", &["v", "vol"], AUDIO, &[req("0..100|+N|-N", &[])], &[], "sink volume", Ready),
    cmd!("mute", &[], AUDIO, &[req("on|off", ON_OFF)], &[], "mute the AirPods sink", Ready),
    cmd!("profile", &["p"], AUDIO,
        &[req("high|headset|off", &["high", "headset", "off"])], &[],
        "bluetooth audio profile", Ready),
    cmd!("codec", &[], AUDIO,
        &[req("sbc|aac|aptx|ldac", &["sbc", "aac", "aptx", "aptx_hd", "ldac"])], &[],
        "A2DP codec", Ready),
    cmd!("default", &["default-sink"], AUDIO, &[], &[],
        "make them the default sink", Ready),
    cmd!("latency", &[], AUDIO, &[req("ms", &[])], &[],
        "output latency offset", Ready),

    cmd!("watch", &["w"], STREAMING, &[], &[], "live event stream", Ready),
    cmd!("meter", &[], STREAMING, &[], "[options]",
        &["--plain", "--json", "--once", "--interval", "--device"],
        "live RMS / peak meter", Ready),
    cmd!("ble", &[], STREAMING, &[opt("on|off", ON_OFF)], &[],
        "listen for the case over LE", Ready),

    cmd!("tray", &[], UI,
        &[req("start|stop|status|restart", &["start", "stop", "status", "restart"])], &[],
        "status-bar icon", Ready),
    cmd!("popup", &[], UI, &[], &[], "show the case bubble now", Ready),
    cmd!("reboot", &[], UI, &[], &[], "restart podctl's services", Ready),

    cmd!("completion", &[], MISC,
        &[req("bash|zsh|fish", &["bash", "zsh", "fish"])], &[],
        "completion script to stdout", Ready),
    cmd!("debug", &[], MISC, &[], "[--no-redact]", &["--no-redact"],
        "diagnostic report", Ready),
    cmd!("install", &[], MISC, &[], "[options]",
        &["--yes", "-y", "--force", "--no-daemon", "--no-completion", "--no-manpages",
          "--no-tray", "--no-popup", "--with-tray", "--with-popup", "--gnome-extension"],
        "install into ~/.local", Ready),
    cmd!("uninstall", &[], MISC, &[], "[--yes]", &["--yes", "-y", "--purge"],
        "remove what install added", Ready),
    cmd!("help", &[], MISC, &[opt("command", &[])], &[],
        "this list, or one command", Ready),
    cmd!("version", &[], MISC, &[], &[], "CLI and daemon version", Ready),
];

/// Flags every verb takes.
pub const GLOBAL_FLAGS: &[&str] = &["--json", "--pretty", "--verbose", "-v", "--help", "-h"];

pub fn find(word: &str) -> Option<&'static Cmd> {
    COMMANDS.iter().find(|c| c.matches(word))
}

/// Every name and alias, for the completions and the man-page test.
pub fn words() -> Vec<&'static str> {
    let mut out = Vec::new();
    for c in COMMANDS {
        out.push(c.name);
        out.extend_from_slice(c.aliases);
    }
    out
}

/// The closest command name to a typo, if one is close enough to be
/// worth suggesting.
pub fn suggest(word: &str) -> Option<&'static str> {
    let lower = word.to_ascii_lowercase();
    let mut best: Option<(usize, &'static str)> = None;
    for name in words() {
        let d = distance(&lower, name);
        // One edit per three characters, so "stauts" finds "status"
        // while "x" does not pick a name out of the air.
        let allowed = (name.len() / 3).clamp(1, 3);
        if d <= allowed && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, name));
        }
    }
    best.map(|(_, name)| name)
}

/// Levenshtein distance, two rows at a time.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::{COMMANDS, GROUPS, State, find, suggest, words};

    #[test]
    fn names_and_aliases_are_unique() {
        let all = words();
        for (i, w) in all.iter().enumerate() {
            assert!(
                !all[i + 1..].contains(w),
                "'{w}' is listed twice in the command table"
            );
        }
    }

    #[test]
    fn every_command_is_in_a_known_group() {
        for c in COMMANDS {
            assert!(GROUPS.contains(&c.group), "{}: group {}", c.name, c.group);
            assert!(!c.summary.is_empty(), "{}: no summary", c.name);
        }
    }

    #[test]
    fn aliases_resolve_to_their_command() {
        assert_eq!(find("s").map(|c| c.name), Some("status"));
        assert_eq!(find("obanc").map(|c| c.name), Some("one-bud-anc"));
        assert_eq!(find("tone").map(|c| c.name), Some("tone-on-press"));
        assert!(find("frobnicate").is_none());
    }

    /// The four settings whose AAP ids are unknown must stay marked, or
    /// the help promises what the daemon then refuses.
    #[test]
    fn the_unimplemented_four_are_marked() {
        let unsupported: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.state == State::NotImplemented)
            .map(|c| c.name)
            .collect();
        assert_eq!(
            unsupported,
            vec!["spatial", "loud-reduction", "press", "tone-on-press"]
        );
    }

    #[test]
    fn typos_find_the_command() {
        assert_eq!(suggest("stauts"), Some("status"));
        assert_eq!(suggest("batery"), Some("battery"));
        assert_eq!(suggest("volum"), Some("volume"));
        assert_eq!(suggest("zzzzzzzz"), None);
    }
}
