//! Whether the session's screen is locked, for callers that must not draw
//! over a lock screen.

/// What each source said. `None` means it did not answer at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sources {
    /// `org.freedesktop.ScreenSaver.GetActive`, i.e. the state right now.
    pub screen_saver: Option<bool>,
    /// logind's `LockedHint`.
    pub locked_hint: Option<bool>,
}

/// The screen saver answers for the moment it is asked. logind's hint is a
/// flag the desktop sets on lock and need not clear on unlock — KDE
/// leaves it standing — so it only answers where there is no screen
/// saver to ask.
pub fn verdict(s: Sources) -> bool {
    s.screen_saver.or(s.locked_hint).unwrap_or(false)
}

/// Ask both sources. Neither failure is fatal: an unanswered source is
/// left at `None` for [`verdict`] to skip.
pub async fn sources() -> Sources {
    Sources {
        screen_saver: screen_saver().await,
        locked_hint: locked_hint().await,
    }
}

/// [`sources`] for callers without a runtime of their own. Panics inside
/// one, like every `block_on`.
pub fn sources_blocking() -> Sources {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt.block_on(sources()),
        Err(_) => Sources::default(),
    }
}

async fn screen_saver() -> Option<bool> {
    let conn = zbus::Connection::session().await.ok()?;
    let p = zbus::Proxy::new(
        &conn,
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
        "org.freedesktop.ScreenSaver",
    )
    .await
    .ok()?;
    p.call_method("GetActive", &())
        .await
        .ok()?
        .body()
        .deserialize::<bool>()
        .ok()
}

async fn locked_hint() -> Option<bool> {
    let conn = zbus::Connection::system().await.ok()?;
    let p = zbus::Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
    )
    .await
    .ok()?;
    p.get_property::<bool>("LockedHint").await.ok()
}

#[cfg(test)]
mod tests {
    use super::{Sources, verdict};

    /// The case podctl got wrong on KDE: the hint stays on after an
    /// unlock.
    #[test]
    fn a_stale_hint_does_not_outvote_the_screen_saver() {
        assert!(!verdict(Sources {
            screen_saver: Some(false),
            locked_hint: Some(true),
        }));
    }

    #[test]
    fn a_locked_screen_saver_counts_whatever_the_hint_says() {
        for hint in [None, Some(false), Some(true)] {
            assert!(verdict(Sources {
                screen_saver: Some(true),
                locked_hint: hint,
            }));
        }
    }

    /// Compositors without a screen saver service: swaylock and the like
    /// set the hint themselves, so there it is the only answer there is.
    #[test]
    fn without_a_screen_saver_the_hint_decides() {
        assert!(verdict(Sources {
            screen_saver: None,
            locked_hint: Some(true),
        }));
        assert!(!verdict(Sources {
            screen_saver: None,
            locked_hint: Some(false),
        }));
    }

    /// A session bus that is not there at all must not lock the bubble out.
    #[test]
    fn no_answer_at_all_is_not_locked() {
        assert!(!verdict(Sources::default()));
    }
}
