use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{debug, warn};
use zbus::zvariant::Value;

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
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
}

const REARM_MARGIN: u8 = 5;

pub struct Notifier {
    proxy: NotificationsProxy<'static>,
    threshold: u8,
    armed: AtomicBool,
}

impl Notifier {
    pub async fn new(conn: &zbus::Connection, threshold: u8) -> zbus::Result<Self> {
        let proxy = NotificationsProxy::new(conn).await?;
        Ok(Self {
            proxy,
            threshold,
            armed: AtomicBool::new(true),
        })
    }

    pub async fn observe(&self, state: &TrayState) {
        if !state.connected {
            return;
        }
        if state.buds_charging() {
            self.armed.store(true, Ordering::SeqCst);
            return;
        }
        let Some(pct) = state.battery_min_buds() else {
            return;
        };
        if pct >= self.threshold.saturating_add(REARM_MARGIN) {
            self.armed.store(true, Ordering::SeqCst);
            return;
        }
        if pct < self.threshold && self.armed.swap(false, Ordering::SeqCst) {
            debug!(pct, threshold = self.threshold, "low-battery notify firing");
            if let Err(e) = self.fire(state, pct).await {
                warn!(error = %e, "send notification");
            }
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

    async fn fire(&self, state: &TrayState, pct: u8) -> zbus::Result<()> {
        let title = state.title();
        let summary = format!("{title} battery low");
        let body = format!("Lowest bud at {pct}%");
        self.proxy
            .notify(
                "podctl-tray",
                0,
                "battery-caution-symbolic",
                &summary,
                &body,
                &[],
                HashMap::new(),
                5_000,
            )
            .await?;
        Ok(())
    }
}
