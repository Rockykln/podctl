//! The AirPods battery where the desktop keeps every other one: BlueZ's
//! battery provider API, which UPower and the panels read.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tracing::{debug, info, warn};
use zbus::{Connection, ObjectPath, OwnedObjectPath, OwnedVariant, Proxy, interface};

use super::Daemon;

const BLUEZ: &str = "org.bluez";
/// Our provider objects live under here. BlueZ walks the root with an
/// ObjectManager, so that is what the root carries.
const ROOT: &str = "/com/rockykln/podctl";
const OBJECT: &str = "/com/rockykln/podctl/battery";
/// What the desktop shows as the origin of the reading, and how we
/// recognise our own entry again.
const SOURCE: &str = "podctl";
/// The state is in memory, so looking at it costs nothing; D-Bus only
/// sees a message when the level actually moves.
const TICK: Duration = Duration::from_secs(2);
/// First and last wait after BlueZ refused to take the provider.
const RETRY_FIRST: Duration = Duration::from_secs(10);
const RETRY_MAX: Duration = Duration::from_secs(300);
/// How many ticks BlueZ gets to publish the battery before we say that
/// it did not.
const PUBLISH_TICKS: u32 = 5;

struct Battery {
    device: OwnedObjectPath,
    percentage: u8,
}

#[interface(name = "org.bluez.BatteryProvider1")]
impl Battery {
    #[zbus(property)]
    fn device(&self) -> ObjectPath<'_> {
        self.device.as_ref()
    }

    #[zbus(property)]
    fn percentage(&self) -> u8 {
        self.percentage
    }

    /// UPower shows this as where the reading came from.
    #[zbus(property)]
    fn source(&self) -> &str {
        SOURCE
    }
}

pub async fn run(daemon: Arc<Daemon>) {
    if !podctl::config::load().battery_provider {
        debug!("battery provider switched off in daemon.toml");
        return;
    }
    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "no system bus — the desktop will not show the battery");
            return;
        }
    };
    if let Err(e) = conn
        .object_server()
        .at(ROOT, zbus::fdo::ObjectManager)
        .await
    {
        warn!(error = %e, "cannot serve the battery provider");
        return;
    }

    // bluetoothd forgets every provider when it restarts, and says so
    // by changing who owns the name.
    let mut restarts = match bluez_restarts(&conn).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "cannot watch for BlueZ restarts — the battery may go missing after one");
            return;
        }
    };

    let mut state = Registration::default();
    let mut slot = Slot::default();
    let mut shown: Option<(String, u8)> = None;
    let mut published: Option<OwnedObjectPath> = None;
    let mut ticker = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            Some(_) = restarts.next() => {
                // Whatever we published went with the old bluetoothd.
                debug!("BlueZ restarted — registering the battery again");
                remove(&conn).await;
                state = Registration::default();
                slot = Slot::default();
                shown = None;
                published = None;
                continue;
            }
        }

        // BlueZ keeps the first battery it is told about for a device
        // and drops a second one with nothing but a line in its own
        // log, so look at whose reading it actually kept.
        if let Some(device) = published.clone()
            && !slot.settled()
        {
            match holder(&conn, &device).await {
                Holder::Ours => slot.ours(),
                Holder::Other(source) => {
                    slot.taken(source.as_deref(), Instant::now());
                    // Give the place back and ask again later: BlueZ
                    // does not come looking for us when the other one
                    // goes away.
                    remove(&conn).await;
                    shown = None;
                    published = None;
                    continue;
                }
                Holder::Nobody => slot.waiting(),
            }
        }

        let want = wanted(&daemon).await;
        if want == shown {
            continue;
        }
        match &want {
            Some((mac, percent)) => {
                let Some(device) = device_path(&conn, mac).await else {
                    // The address is ours but BlueZ has no object for
                    // it: drop what we published rather than leave a
                    // level standing that belongs to nothing.
                    if shown.is_some() {
                        debug!(%mac, "BlueZ dropped the device — taking the battery back");
                        remove(&conn).await;
                        shown = None;
                        published = None;
                        slot = Slot::default();
                    }
                    continue;
                };
                if shown.is_some() && published.as_ref() != Some(&device) {
                    remove(&conn).await;
                    slot = Slot::default();
                }
                let now = Instant::now();
                if !state.may_try(now) || !slot.may_try(now) {
                    continue;
                }
                if let Err(e) = publish(&conn, device.clone(), *percent).await {
                    warn!(error = %e, "cannot publish the battery");
                    continue;
                }
                if !state.registered {
                    match register(&conn).await {
                        Ok(()) => {
                            state.registered = true;
                            state.wait = RETRY_FIRST;
                            info!("battery registered with BlueZ");
                        }
                        Err(e) => {
                            // Older BlueZ, a policy that says no, or an
                            // adapter that is not there yet. Say it
                            // once, then try again now and then.
                            if !state.complained {
                                warn!(error = %e, "BlueZ took no battery provider — trying again later");
                                state.complained = true;
                            }
                            state.back_off(now);
                            remove(&conn).await;
                            continue;
                        }
                    }
                }
                published = Some(device);
            }
            None => {
                if shown.is_some() {
                    remove(&conn).await;
                    published = None;
                    slot = Slot::default();
                }
            }
        }
        shown = want;
    }
}

/// Keeps a refused registration from being retried every two seconds.
struct Registration {
    registered: bool,
    complained: bool,
    wait: Duration,
    next_try: Option<Instant>,
}

impl Default for Registration {
    fn default() -> Self {
        Self {
            registered: false,
            complained: false,
            wait: RETRY_FIRST,
            next_try: None,
        }
    }
}

impl Registration {
    fn may_try(&self, now: Instant) -> bool {
        self.registered || self.next_try.is_none_or(|t| now >= t)
    }

    fn back_off(&mut self, now: Instant) {
        self.next_try = Some(now + self.wait);
        self.wait = (self.wait * 2).min(RETRY_MAX);
    }
}

/// What became of the battery we handed to BlueZ.
#[derive(Default)]
struct Slot {
    confirmed: bool,
    /// Said once per holder, so a headset that another program reports
    /// does not fill the log.
    told: bool,
    waited: u32,
    retry_at: Option<Instant>,
    wait: Duration,
}

impl Slot {
    /// Nothing left to look at: either it is ours or we gave up asking.
    fn settled(&self) -> bool {
        self.confirmed
    }

    fn ours(&mut self) {
        self.confirmed = true;
        self.waited = 0;
        info!("the desktop shows the battery");
    }

    fn taken(&mut self, source: Option<&str>, now: Instant) {
        if !self.told {
            match source {
                Some(s) => {
                    warn!(source = %s, "another program already reports this battery to BlueZ")
                }
                None => warn!("something else already reports this battery to BlueZ"),
            }
            self.told = true;
        }
        self.waited = 0;
        self.wait = if self.wait.is_zero() {
            RETRY_FIRST
        } else {
            (self.wait * 2).min(RETRY_MAX)
        };
        self.retry_at = Some(now + self.wait);
    }

    fn waiting(&mut self) {
        self.waited += 1;
        if self.waited == PUBLISH_TICKS && !self.told {
            warn!("BlueZ took the battery but published nothing — the desktop will not show it");
            self.told = true;
        }
    }

    fn may_try(&self, now: Instant) -> bool {
        self.retry_at.is_none_or(|t| now >= t)
    }
}

/// Who holds the battery BlueZ shows for a device.
enum Holder {
    Ours,
    Other(Option<String>),
    Nobody,
}

async fn holder(conn: &Connection, device: &OwnedObjectPath) -> Holder {
    let Ok(proxy) = Proxy::new(conn, BLUEZ, device.as_ref(), "org.bluez.Battery1").await else {
        return Holder::Nobody;
    };
    // Percentage says whether the interface is there at all; Source is
    // optional, so a battery without one is not ours either.
    if proxy.get_property::<u8>("Percentage").await.is_err() {
        return Holder::Nobody;
    }
    match proxy.get_property::<String>("Source").await {
        Ok(s) if s == SOURCE => Holder::Ours,
        Ok(s) => Holder::Other(Some(s)),
        Err(_) => Holder::Other(None),
    }
}

async fn bluez_restarts(conn: &Connection) -> zbus::Result<zbus::fdo::NameOwnerChangedStream> {
    zbus::fdo::DBusProxy::new(conn)
        .await?
        .receive_name_owner_changed_with_args(&[(0, BLUEZ)])
        .await
}

async fn wanted(daemon: &Arc<Daemon>) -> Option<(String, u8)> {
    let state = daemon.state.read().await;
    reportable(&state)
}

/// What the desktop should see: the emptier bud, which is what a phone
/// shows too. `None` while there is nothing to say — a battery left
/// behind after a disconnect would be a lie the panel keeps showing.
fn reportable(s: &podctl::DeviceState) -> Option<(String, u8)> {
    if !s.connected {
        return None;
    }
    let mac = s.address.clone()?;
    let percent = [s.battery.left, s.battery.right]
        .into_iter()
        .flatten()
        .min()?;
    Some((mac, percent))
}

async fn publish(conn: &Connection, device: OwnedObjectPath, percentage: u8) -> zbus::Result<()> {
    let server = conn.object_server();
    if let Ok(iface) = server.interface::<_, Battery>(OBJECT).await {
        let emitter = iface.signal_emitter().clone();
        let mut battery = iface.get_mut().await;
        if battery.percentage != percentage {
            battery.percentage = percentage;
            battery.percentage_changed(&emitter).await?;
        }
        return Ok(());
    }
    server
        .at(OBJECT, Battery { device, percentage })
        .await
        .map(|_| ())
}

async fn remove(conn: &Connection) {
    let _ = conn.object_server().remove::<Battery, _>(OBJECT).await;
}

async fn register(conn: &Connection) -> zbus::Result<()> {
    let adapters = adapters(conn).await;
    let mut last = None;
    for adapter in adapters {
        let proxy = Proxy::new(
            conn,
            BLUEZ,
            adapter.as_ref(),
            "org.bluez.BatteryProviderManager1",
        )
        .await?;
        let root = ObjectPath::try_from(ROOT)?;
        match proxy.call_method("RegisterBatteryProvider", &(root,)).await {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| zbus::Error::Failure("no Bluetooth adapter".into())))
}

async fn adapters(conn: &Connection) -> Vec<OwnedObjectPath> {
    managed(conn)
        .await
        .into_iter()
        .filter(|(_, ifaces)| ifaces.contains_key("org.bluez.Adapter1"))
        .map(|(path, _)| path)
        .collect()
}

/// The device object BlueZ keeps for this address. Built from the
/// object tree rather than from the MAC, so a second adapter does not
/// send the battery to the wrong path.
async fn device_path(conn: &Connection, mac: &str) -> Option<OwnedObjectPath> {
    let want = mac.to_ascii_uppercase();
    for (path, ifaces) in managed(conn).await {
        let Some(props) = ifaces.get("org.bluez.Device1") else {
            continue;
        };
        let address: Option<String> = props
            .get("Address")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| v.try_into().ok());
        if address.is_some_and(|a| a.to_ascii_uppercase() == want) {
            return Some(path);
        }
    }
    None
}

type Managed = Vec<(
    OwnedObjectPath,
    HashMap<String, HashMap<String, OwnedVariant>>,
)>;

async fn managed(conn: &Connection) -> Managed {
    let Ok(proxy) = Proxy::new(conn, BLUEZ, "/", "org.freedesktop.DBus.ObjectManager").await else {
        return Vec::new();
    };
    match proxy.call_method("GetManagedObjects", &()).await {
        Ok(reply) => reply
            .body()
            .deserialize::<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedVariant>>>>()
            .map(|m| m.into_iter().collect())
            .unwrap_or_default(),
        Err(e) => {
            debug!(error = %e, "BlueZ listed no objects");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PUBLISH_TICKS, RETRY_FIRST, Slot, reportable};
    use podctl::DeviceState;
    use std::time::Instant;

    fn state(connected: bool, left: Option<u8>, right: Option<u8>) -> DeviceState {
        let mut s = DeviceState {
            connected,
            address: Some("AA:BB:CC:DD:EE:01".into()),
            ..DeviceState::default()
        };
        s.battery.left = left;
        s.battery.right = right;
        s.battery.case = Some(90);
        s
    }

    #[test]
    fn the_emptier_bud_is_what_the_panel_gets() {
        assert_eq!(
            reportable(&state(true, Some(84), Some(78))),
            Some(("AA:BB:CC:DD:EE:01".to_string(), 78))
        );
        // One bud in the case still reports; the other one counts.
        assert_eq!(
            reportable(&state(true, None, Some(63))),
            Some(("AA:BB:CC:DD:EE:01".to_string(), 63))
        );
    }

    #[test]
    fn nothing_is_reported_without_a_link_or_a_level() {
        assert_eq!(reportable(&state(false, Some(84), Some(78))), None);
        assert_eq!(reportable(&state(true, None, None)), None);
        let mut no_address = state(true, Some(84), None);
        no_address.address = None;
        assert_eq!(reportable(&no_address), None);
    }

    #[test]
    fn the_case_alone_is_not_a_headset_battery() {
        // The case announces itself over LE while the buds are away:
        // that is not the device BlueZ knows.
        let s = state(false, None, None);
        assert!(s.battery.case.is_some());
        assert_eq!(reportable(&s), None);
    }

    #[test]
    fn a_battery_held_by_someone_else_is_asked_for_again_later() {
        let start = Instant::now();
        let mut slot = Slot::default();
        assert!(slot.may_try(start));
        slot.taken(Some("HFP 1.7"), start);
        assert!(!slot.may_try(start), "we just gave the place back");
        assert!(!slot.settled());
        assert!(
            slot.may_try(start + RETRY_FIRST),
            "after the wait we try to take it again"
        );
        // The second refusal waits longer than the first.
        slot.taken(Some("HFP 1.7"), start);
        assert!(!slot.may_try(start + RETRY_FIRST));
    }

    #[test]
    fn ours_stops_the_looking_and_a_silent_bluez_is_reported() {
        let mut slot = Slot::default();
        slot.ours();
        assert!(slot.settled());
        assert!(slot.may_try(Instant::now()));

        let mut quiet = Slot::default();
        for _ in 0..PUBLISH_TICKS {
            quiet.waiting();
        }
        assert!(quiet.told, "a BlueZ that publishes nothing is said once");
        assert!(!quiet.settled());
    }
}
