//! The desktop's light/dark preference, read from the settings portal.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::StreamExt;
use tracing::{debug, info};
use zbus::zvariant::{OwnedValue, Value};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.Settings";
const NS: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// 1 asks for dark. 0 ("no preference") is what GNOME's default style
/// reports, and that style is light.
fn prefers_dark(v: &Value) -> Option<bool> {
    match v {
        Value::Value(inner) => prefers_dark(inner),
        Value::U32(n) => Some(*n == 1),
        _ => None,
    }
}

async fn proxy(conn: &zbus::Connection) -> zbus::Result<zbus::Proxy<'static>> {
    zbus::Proxy::new(conn, DEST, PATH, IFACE).await
}

async fn read(p: &zbus::Proxy<'_>) -> Option<bool> {
    // ReadOne is portal v2; older portals only have Read, which wraps
    // the value in one more variant.
    let reply = match p.call_method("ReadOne", &(NS, KEY)).await {
        Ok(m) => m,
        Err(_) => p.call_method("Read", &(NS, KEY)).await.ok()?,
    };
    let v: OwnedValue = reply.body().deserialize().ok()?;
    prefers_dark(&v)
}

/// The preference right now; `None` without a portal.
pub async fn current() -> Option<bool> {
    let conn = zbus::Connection::session().await.ok()?;
    read(&proxy(&conn).await.ok()?).await
}

/// Keeps `dark` in step with the desktop until the session bus goes away.
/// Without a portal it stays as it was set.
pub async fn follow(dark: Arc<AtomicBool>) {
    let Ok(conn) = zbus::Connection::session().await else {
        return;
    };
    let Ok(p) = proxy(&conn).await else { return };
    let Ok(mut changes) = p.receive_signal("SettingChanged").await else {
        return;
    };
    match read(&p).await {
        Some(d) => dark.store(d, Ordering::Relaxed),
        None => {
            debug!("no color-scheme from the settings portal — staying dark");
            return;
        }
    }
    while let Some(msg) = changes.next().await {
        let Ok((ns, key, v)) = msg.body().deserialize::<(String, String, OwnedValue)>() else {
            continue;
        };
        if ns != NS || key != KEY {
            continue;
        }
        // KDE sends the same change several times over.
        if let Some(d) = prefers_dark(&v)
            && dark.swap(d, Ordering::Relaxed) != d
        {
            info!(dark = d, "desktop color scheme changed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_portal_values() {
        assert_eq!(prefers_dark(&Value::U32(1)), Some(true));
        assert_eq!(prefers_dark(&Value::U32(2)), Some(false));
        assert_eq!(prefers_dark(&Value::U32(0)), Some(false));
        let wrapped = Value::Value(Box::new(Value::Value(Box::new(Value::U32(1)))));
        assert_eq!(prefers_dark(&wrapped), Some(true));
        assert_eq!(prefers_dark(&Value::Str("dark".into())), None);
    }
}
