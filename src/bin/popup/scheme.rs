//! The desktop's light/dark and motion preferences, read from the
//! settings portal.

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

async fn read_value(p: &zbus::Proxy<'_>, ns: &str, key: &str) -> Option<OwnedValue> {
    // ReadOne is portal v2; older portals only have Read, which wraps
    // the value in one more variant.
    let reply = match p.call_method("ReadOne", &(ns, key)).await {
        Ok(m) => m,
        Err(_) => p.call_method("Read", &(ns, key)).await.ok()?,
    };
    reply.body().deserialize().ok()
}

async fn read(p: &zbus::Proxy<'_>) -> Option<bool> {
    let v = read_value(p, NS, KEY).await?;
    prefers_dark(&v)
}

/// The places a desktop says "less motion": the portal's own key, GNOME's
/// animations switch, KDE's animation speed at zero.
const MOTION: [(&str, &str); 3] = [
    ("org.freedesktop.appearance", "reduced-motion"),
    ("org.gnome.desktop.interface", "enable-animations"),
    ("org.kde.kdeglobals.KDE", "AnimationDurationFactor"),
];

fn asks_less_motion(ns: &str, key: &str, v: &Value) -> Option<bool> {
    match (ns, key, v) {
        (_, _, Value::Value(inner)) => asks_less_motion(ns, key, inner),
        ("org.freedesktop.appearance", "reduced-motion", Value::U32(n)) => Some(*n == 1),
        ("org.gnome.desktop.interface", "enable-animations", Value::Bool(on)) => Some(!on),
        ("org.kde.kdeglobals.KDE", "AnimationDurationFactor", Value::Str(f)) => {
            f.as_str().trim().parse::<f32>().ok().map(|f| f == 0.0)
        }
        _ => None,
    }
}

/// Any one place asking for less motion is enough.
#[derive(Default)]
struct Motion([Option<bool>; 3]);

impl Motion {
    fn set(&mut self, ns: &str, key: &str, v: &Value) -> bool {
        if let Some(i) = MOTION.iter().position(|&(n, k)| n == ns && k == key)
            && let Some(r) = asks_less_motion(ns, key, v)
        {
            self.0[i] = Some(r);
            return true;
        }
        false
    }

    fn reduced(&self) -> bool {
        self.0.contains(&Some(true))
    }
}

async fn read_motion(p: &zbus::Proxy<'_>) -> Motion {
    let mut m = Motion::default();
    for (ns, key) in MOTION {
        if let Some(v) = read_value(p, ns, key).await {
            m.set(ns, key, &v);
        }
    }
    m
}

/// Whether the desktop asks for less motion right now; false without a
/// portal.
pub async fn reduced_motion() -> bool {
    let Ok(conn) = zbus::Connection::session().await else {
        return false;
    };
    match proxy(&conn).await {
        Ok(p) => read_motion(&p).await.reduced(),
        Err(_) => false,
    }
}

/// The preference right now; `None` without a portal.
pub async fn current() -> Option<bool> {
    let conn = zbus::Connection::session().await.ok()?;
    read(&proxy(&conn).await.ok()?).await
}

/// Keeps `dark` and `reduced` (where given) in step with the desktop
/// until the session bus goes away. Without a portal they stay as set.
pub async fn follow(dark: Option<Arc<AtomicBool>>, reduced: Option<Arc<AtomicBool>>) {
    let Ok(conn) = zbus::Connection::session().await else {
        return;
    };
    let Ok(p) = proxy(&conn).await else { return };
    let Ok(mut changes) = p.receive_signal("SettingChanged").await else {
        return;
    };
    if let Some(dark) = &dark {
        match read(&p).await {
            Some(d) => dark.store(d, Ordering::Relaxed),
            None => debug!("no color-scheme from the settings portal — staying dark"),
        }
    }
    let mut motion = read_motion(&p).await;
    if let Some(reduced) = &reduced {
        reduced.store(motion.reduced(), Ordering::Relaxed);
    }
    while let Some(msg) = changes.next().await {
        let Ok((ns, key, v)) = msg.body().deserialize::<(String, String, OwnedValue)>() else {
            continue;
        };
        // KDE sends the same change several times over.
        if let Some(dark) = &dark
            && ns == NS
            && key == KEY
            && let Some(d) = prefers_dark(&v)
            && dark.swap(d, Ordering::Relaxed) != d
        {
            info!(dark = d, "desktop color scheme changed");
        }
        if let Some(reduced) = &reduced
            && motion.set(&ns, &key, &v)
        {
            let r = motion.reduced();
            if reduced.swap(r, Ordering::Relaxed) != r {
                info!(reduced = r, "desktop motion preference changed");
            }
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

    #[test]
    fn reads_the_motion_settings() {
        let mut m = Motion::default();
        assert!(!m.reduced());
        let (portal, gnome, kde) = (MOTION[0], MOTION[1], MOTION[2]);
        assert!(m.set(kde.0, kde.1, &Value::Str("0.25".into())));
        assert!(m.set(gnome.0, gnome.1, &Value::Bool(true)));
        assert!(m.set(portal.0, portal.1, &Value::U32(0)));
        assert!(!m.reduced());
        m.set(
            kde.0,
            kde.1,
            &Value::Value(Box::new(Value::Str("0".into()))),
        );
        assert!(m.reduced());
        m.set(kde.0, kde.1, &Value::Str("1".into()));
        m.set(gnome.0, gnome.1, &Value::Bool(false));
        assert!(m.reduced());
        assert!(!m.set("org.example", "x", &Value::U32(1)));
    }
}
