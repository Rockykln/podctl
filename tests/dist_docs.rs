//! Guards on the hand-written files under `dist/` — man pages, shell
//! completions, systemd units. Nothing compiles them, so drift against
//! the CLI is otherwise only found by a user.

const MAN_PODCTL: &str = include_str!("../dist/podctl.1");
const MAN_PODCTLD: &str = include_str!("../dist/podctld.1");
const BASH: &str = include_str!("../dist/completion/podctl.bash");
const ZSH: &str = include_str!("../dist/completion/_podctl.zsh");
const FISH: &str = include_str!("../dist/completion/podctl.fish");

/// Canonical names, from the one table the parser uses.
fn verbs() -> Vec<&'static str> {
    podctl::commands::COMMANDS.iter().map(|c| c.name).collect()
}

/// The `.TH` line carries the version users see in `man podctl`. It was
/// was left at 0.1.0 through the whole 0.1.1 release; this keeps it
/// honest, and catches a release bump that forgot the man pages.
#[test]
fn man_pages_carry_the_crate_version() {
    let want = format!("\"podctl {}\"", env!("CARGO_PKG_VERSION"));
    for (name, text) in [("podctl.1", MAN_PODCTL), ("podctld.1", MAN_PODCTLD)] {
        let th = text.lines().next().unwrap_or_default();
        assert!(
            th.starts_with(".TH"),
            "{name}: first line should be the .TH header, got {th:?}"
        );
        assert!(
            th.contains(&want),
            "{name}: .TH says {th:?}, expected it to contain {want}"
        );
    }
}

#[test]
fn completions_offer_every_command() {
    for (name, text) in [
        ("podctl.bash", BASH),
        ("_podctl.zsh", ZSH),
        ("podctl.fish", FISH),
    ] {
        let missing: Vec<&str> = verbs().into_iter().filter(|v| !mentions(text, v)).collect();
        assert!(missing.is_empty(), "{name} does not offer: {missing:?}");
    }
}

/// The files under `dist/` are what packagers install, and they are
/// written out by `podctl completion`. If the table changes, they have
/// to be regenerated — that is this test.
#[test]
fn the_shipped_completions_are_the_generated_ones() {
    for (name, text, shell) in [
        ("podctl.bash", BASH, "bash"),
        ("_podctl.zsh", ZSH, "zsh"),
        ("podctl.fish", FISH, "fish"),
    ] {
        let generated = podctl::completions::script(shell).expect("known shell");
        assert_eq!(
            text, generated,
            "dist/completion/{name} is out of date — run 'podctl completion {shell}'"
        );
    }
}

#[test]
fn man_page_documents_every_command() {
    // `help` and `version` appear as `.B version, --version, -V` style
    // entries, so a plain substring check is what fits here.
    let missing: Vec<&str> = verbs()
        .into_iter()
        .filter(|v| !mentions(MAN_PODCTL, v))
        .collect();
    assert!(
        missing.is_empty(),
        "podctl.1 does not document: {missing:?}"
    );
}

/// Aliases belong in the man page too: `podctl dc` and `podctl obanc`
/// worked for releases without being written down anywhere.
#[test]
fn man_page_lists_every_alias() {
    let missing: Vec<&str> = podctl::commands::COMMANDS
        .iter()
        .flat_map(|c| c.aliases.iter().copied())
        .filter(|a| !mentions(MAN_PODCTL, a))
        .collect();
    assert!(missing.is_empty(), "podctl.1 does not mention: {missing:?}");
}

/// Whole-word match, so `default` isn't satisfied by `default-sink` and
/// `ear` isn't satisfied by `linear`.
fn mentions(haystack: &str, word: &str) -> bool {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let mut from = 0;
    while let Some(i) = haystack[from..].find(word) {
        let start = from + i;
        let end = start + word.len();
        let before_ok = start == 0 || !is_word(haystack[..start].chars().next_back().unwrap());
        let after_ok = end == haystack.len() || !is_word(haystack[end..].chars().next().unwrap());
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

#[test]
fn mentions_is_whole_word() {
    assert!(mentions("a default b", "default"));
    assert!(!mentions("default-sink only", "default"));
    assert!(!mentions("linear", "ear"));
    assert!(mentions("podctl ear on", "ear"));
}
