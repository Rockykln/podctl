use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tracing::{debug, warn};
use zbus::Variant;

use podctl::model::Battery;

use crate::state::TrayState;

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
pub trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Variant<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
}

/// A bud has to climb this far above a level before it can warn again.
const REARM_MARGIN: u8 = 5;
/// Charging this long counts as charged, whatever the level says. A
/// bud dropped into the case for a moment does not.
const REARM_CHARGE: Duration = Duration::from_secs(120);

static NOTIFIER: OnceLock<Arc<Notifier>> = OnceLock::new();

pub fn install(n: Arc<Notifier>) {
    let _ = NOTIFIER.set(n);
}

pub fn get() -> Option<&'static Notifier> {
    NOTIFIER.get().map(|n| n.as_ref())
}

pub struct Notifier {
    proxy: NotificationsProxy<'static>,
    alarm: Mutex<Alarm>,
    battery_id: AtomicU32,
    failure_id: AtomicU32,
}

impl Notifier {
    pub async fn new(conn: &zbus::Connection, low: u8, critical: u8) -> zbus::Result<Self> {
        let proxy = NotificationsProxy::new(conn).await?;
        Ok(Self {
            proxy,
            alarm: Mutex::new(Alarm::new(low, critical)),
            battery_id: AtomicU32::new(0),
            failure_id: AtomicU32::new(0),
        })
    }

    pub async fn observe(&self, state: &TrayState) {
        if !state.connected {
            return;
        }
        let level = {
            let mut alarm = match self.alarm.lock() {
                Ok(a) => a,
                Err(poisoned) => poisoned.into_inner(),
            };
            alarm.observe(&state.battery, Instant::now())
        };
        let Some(level) = level else { return };
        debug!(?level, "low-battery notify firing");
        if let Err(e) = self.battery_low(state, level).await {
            warn!(error = %e, "send notification");
        }
    }

    /// A daemon of another version makes everything here fail silently,
    /// so it is worth a notification.
    pub async fn version_mismatch(&self, note: &str) -> zbus::Result<()> {
        self.proxy
            .notify(
                "podctl-tray",
                0,
                "dialog-warning-symbolic",
                "podctl needs a restart",
                note,
                &[],
                HashMap::new(),
                10_000,
            )
            .await?;
        Ok(())
    }

    /// Something asked for from the icon or the menu did not happen.
    pub async fn failed(&self, what: &str, why: &str) {
        let hints = HashMap::from([
            ("urgency", Variant::U8(1)),
            ("category", Variant::from("device.error")),
        ]);
        let id = self.failure_id.load(Ordering::SeqCst);
        match self
            .proxy
            .notify(
                "podctl-tray",
                id,
                "dialog-warning-symbolic",
                &format!("{what} failed"),
                why,
                &[],
                hints,
                6_000,
            )
            .await
        {
            Ok(id) => self.failure_id.store(id, Ordering::SeqCst),
            Err(e) => warn!(error = %e, "send notification"),
        }
    }

    async fn battery_low(&self, state: &TrayState, level: Level) -> zbus::Result<()> {
        let (summary, urgency, timeout) = match level {
            Level::Critical => (format!("{}: battery critically low", state.title()), 2, -1),
            _ => (format!("{}: battery low", state.title()), 1, 10_000),
        };
        let hints = HashMap::from([
            ("urgency", Variant::U8(urgency)),
            ("category", Variant::from("device")),
        ]);
        let id = self.battery_id.load(Ordering::SeqCst);
        let id = self
            .proxy
            .notify(
                "podctl-tray",
                id,
                "battery-caution-symbolic",
                &summary,
                &bud_levels(&state.battery),
                &[],
                hints,
                timeout,
            )
            .await?;
        self.battery_id.store(id, Ordering::SeqCst);
        Ok(())
    }
}

/// "Left AirPod at 9% · Right 34%", the lower one first.
fn bud_levels(b: &Battery) -> String {
    let mut buds: Vec<(&str, u8)> = [("Left", b.left), ("Right", b.right)]
        .into_iter()
        .filter_map(|(side, pct)| pct.map(|p| (side, p)))
        .collect();
    buds.sort_by_key(|&(_, p)| p);
    let mut parts = buds.into_iter();
    let Some((side, pct)) = parts.next() else {
        return String::new();
    };
    let mut out = format!("{side} AirPod at {pct}%");
    for (side, pct) in parts {
        out.push_str(&format!(" · {side} {pct}%"));
    }
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    #[default]
    Fine,
    Low,
    Critical,
}

/// When to warn about a bud running low. Each bud keeps its own memory
/// of what it already warned about, so the other one going into the
/// case does not set off the same warning again.
pub struct Alarm {
    low: u8,
    critical: u8,
    left: Bud,
    right: Bud,
}

#[derive(Default)]
struct Bud {
    warned: Level,
    charging_since: Option<Instant>,
}

impl Alarm {
    pub fn new(low: u8, critical: u8) -> Self {
        Self {
            low,
            critical: critical.min(low),
            left: Bud::default(),
            right: Bud::default(),
        }
    }

    /// The level to announce now, if either bud crossed into a new one.
    pub fn observe(&mut self, b: &Battery, now: Instant) -> Option<Level> {
        let (low, critical) = (self.low, self.critical);
        let l = self
            .left
            .observe(b.left, b.left_charging, low, critical, now);
        let r = self
            .right
            .observe(b.right, b.right_charging, low, critical, now);
        l.max(r).filter(|&lvl| lvl > Level::Fine)
    }
}

impl Bud {
    fn observe(
        &mut self,
        pct: Option<u8>,
        charging: bool,
        low: u8,
        critical: u8,
        now: Instant,
    ) -> Option<Level> {
        let pct = pct?;
        if charging {
            let since = *self.charging_since.get_or_insert(now);
            if now.duration_since(since) >= REARM_CHARGE {
                self.warned = Level::Fine;
            }
            return None;
        }
        self.charging_since = None;
        if pct >= low.saturating_add(REARM_MARGIN) {
            self.warned = Level::Fine;
        } else if pct >= critical.saturating_add(REARM_MARGIN) && self.warned == Level::Critical {
            self.warned = Level::Low;
        }
        let level = if pct < critical {
            Level::Critical
        } else if pct < low {
            Level::Low
        } else {
            Level::Fine
        };
        if level > self.warned {
            self.warned = level;
            Some(level)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buds(l: u8, r: u8) -> Battery {
        Battery {
            left: Some(l),
            right: Some(r),
            ..Battery::default()
        }
    }

    #[test]
    fn each_level_warns_once_on_the_way_down() {
        let mut a = Alarm::new(20, 10);
        let t = Instant::now();
        assert_eq!(a.observe(&buds(40, 40), t), None);
        assert_eq!(a.observe(&buds(19, 40), t), Some(Level::Low));
        assert_eq!(a.observe(&buds(15, 40), t), None);
        assert_eq!(a.observe(&buds(9, 40), t), Some(Level::Critical));
        assert_eq!(a.observe(&buds(5, 40), t), None);
    }

    #[test]
    fn a_short_visit_to_the_case_does_not_warn_again() {
        let mut a = Alarm::new(20, 10);
        let t = Instant::now();
        assert_eq!(a.observe(&buds(9, 40), t), Some(Level::Critical));
        let mut charging = buds(9, 40);
        charging.left_charging = true;
        assert_eq!(a.observe(&charging, t + Duration::from_secs(5)), None);
        assert_eq!(a.observe(&buds(9, 40), t + Duration::from_secs(30)), None);
    }

    #[test]
    fn charging_for_a_while_or_climbing_rearms() {
        let mut a = Alarm::new(20, 10);
        let t = Instant::now();
        assert_eq!(a.observe(&buds(9, 40), t), Some(Level::Critical));
        let mut charging = buds(9, 40);
        charging.left_charging = true;
        a.observe(&charging, t);
        a.observe(&charging, t + REARM_CHARGE);
        assert_eq!(
            a.observe(&buds(9, 40), t + REARM_CHARGE),
            Some(Level::Critical)
        );

        let mut b = Alarm::new(20, 10);
        assert_eq!(b.observe(&buds(18, 40), t), Some(Level::Low));
        b.observe(&buds(25, 40), t);
        assert_eq!(b.observe(&buds(18, 40), t), Some(Level::Low));
    }

    #[test]
    fn the_other_bud_has_its_own_memory() {
        let mut a = Alarm::new(20, 10);
        let t = Instant::now();
        assert_eq!(a.observe(&buds(18, 40), t), Some(Level::Low));
        assert_eq!(a.observe(&buds(18, 17), t), Some(Level::Low));
        assert_eq!(a.observe(&buds(18, 17), t), None);
    }

    #[test]
    fn the_text_names_the_low_bud_first() {
        assert_eq!(bud_levels(&buds(34, 9)), "Right AirPod at 9% · Left 34%");
        let one = Battery {
            left: Some(8),
            ..Battery::default()
        };
        assert_eq!(bud_levels(&one), "Left AirPod at 8%");
    }
}
