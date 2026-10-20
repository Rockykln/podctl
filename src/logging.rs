//! One subscriber for every binary. Under systemd journald stamps and
//! colours the lines itself, so printing our own only doubles them.

use tracing_subscriber::EnvFilter;

/// Which variable wins. `PODCTL_LOG` comes first because a
/// `RUST_LOG=trace` left behind in a shell used to make every podctl
/// process started from it chatty at once.
fn chosen<'a>(podctl_log: Option<&'a str>, rust_log: Option<&'a str>, default: &'a str) -> &'a str {
    [podctl_log, rust_log]
        .into_iter()
        .flatten()
        .find(|v| !v.is_empty())
        .unwrap_or(default)
}

fn filter(default: &str) -> EnvFilter {
    let podctl_log = std::env::var("PODCTL_LOG").ok();
    let rust_log = std::env::var("RUST_LOG").ok();
    EnvFilter::new(chosen(podctl_log.as_deref(), rust_log.as_deref(), default))
}

/// Logs go to stderr at `default_level`, unless one of the variables
/// says otherwise. Called once, at startup.
pub fn init(default_level: &str) {
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter(default_level))
        .with_writer(std::io::stderr);
    // journald stamps and colours the lines itself; ours on top are noise.
    if stderr_is_journal() {
        builder.without_time().with_ansi(false).init();
    } else {
        builder.init();
    }
}

/// systemd names the journal stream it handed us in `JOURNAL_STREAM` as
/// `device:inode`. A terminal started from a unit inherits the variable
/// although its own stderr is the terminal, so the question is not
/// whether it is set but whether it describes *our* stderr.
fn stderr_is_journal() -> bool {
    use std::os::fd::AsFd;
    use std::os::unix::fs::MetadataExt;

    let Ok(value) = std::env::var("JOURNAL_STREAM") else {
        return false;
    };
    let Some((dev, ino)) = parse_stream(&value) else {
        return false;
    };
    let Ok(fd) = std::io::stderr().as_fd().try_clone_to_owned() else {
        return false;
    };
    // The clone is what gets closed again, not stderr itself.
    let Ok(meta) = std::fs::File::from(fd).metadata() else {
        return false;
    };
    meta.dev() == dev && meta.ino() == ino
}

fn parse_stream(value: &str) -> Option<(u64, u64)> {
    let (dev, ino) = value.split_once(':')?;
    Some((dev.parse().ok()?, ino.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::{chosen, parse_stream};

    #[test]
    fn podctl_log_wins_over_rust_log() {
        assert_eq!(chosen(Some("debug"), Some("trace"), "info"), "debug");
        assert_eq!(chosen(None, Some("trace"), "info"), "trace");
        assert_eq!(chosen(None, None, "info"), "info");
    }

    /// `RUST_LOG=` is how a unit file unsets it, and an empty filter
    /// silences everything instead of falling back.
    #[test]
    fn an_empty_variable_does_not_count() {
        assert_eq!(chosen(Some(""), Some("warn"), "info"), "warn");
        assert_eq!(chosen(Some(""), Some(""), "info"), "info");
    }

    #[test]
    fn a_journal_stream_is_a_device_and_an_inode() {
        assert_eq!(parse_stream("10:149793"), Some((10, 149793)));
        assert_eq!(parse_stream("10"), None);
        assert_eq!(parse_stream("tty:1"), None);
    }
}
