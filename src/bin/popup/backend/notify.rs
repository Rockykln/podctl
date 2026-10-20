use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use zbus::zvariant::Value;

use super::{Backend, Frame, fingerprint};
use crate::render;

const SVC: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const DEFAULT_EXPIRE_MS: i32 = 6_500;

pub struct Notify {
    w: u32,
    h: u32,
    img: PathBuf,
    id: u32,
    fp: u64,
    expire_ms: i32,
    title: String,
    body: String,
    rt: Option<tokio::runtime::Runtime>,
    conn: Option<zbus::Connection>,
}

impl Notify {
    pub fn new() -> Self {
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        Notify {
            w: 0,
            h: 0,
            img: PathBuf::from(dir).join("podctl-popup.png"),
            id: 0,
            fp: 0,
            expire_ms: DEFAULT_EXPIRE_MS,
            title: "AirPods".into(),
            body: String::new(),
            rt: None,
            conn: None,
        }
    }

    /// Runtime and bus connection are kept: a bubble sends several
    /// updates, and building both again for each one is a round trip
    /// to the bus for nothing.
    fn session(&mut self) -> Result<(&tokio::runtime::Runtime, &zbus::Connection)> {
        if self.rt.is_none() {
            self.rt = Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("tokio runtime")?,
            );
        }
        let rt = self.rt.as_ref().expect("runtime just built");
        if self.conn.is_none() {
            self.conn = Some(
                rt.block_on(zbus::Connection::session())
                    .context("session bus")?,
            );
        }
        Ok((rt, self.conn.as_ref().expect("connection just built")))
    }
}

impl Backend for Notify {
    fn kind(&self) -> &'static str {
        "notify"
    }

    fn set_hold(&mut self, hold: Duration) {
        // Round up: expiring a hair before the hold loop closes the
        // notification ourselves would flash the bubble out early.
        self.expire_ms = hold.as_millis().min(i32::MAX as u128) as i32 + 250;
    }

    fn set_labels(&mut self, title: &str, body: &str) {
        self.title = if title.is_empty() {
            "AirPods".into()
        } else {
            title.to_string()
        };
        self.body = body.to_string();
    }

    fn open(&mut self, w: u32, h: u32) -> Result<()> {
        self.w = w;
        self.h = h;
        Ok(())
    }

    fn push_frame(&mut self, f: &Frame) -> Result<()> {
        let fp = fingerprint(f.bgra);
        if self.id != 0 && fp == self.fp {
            return Ok(());
        }

        let png =
            render::png_from_bgra(f.bgra, self.w, self.h).context("re-encode frame as PNG")?;
        std::fs::write(&self.img, &png).context("write notification image")?;

        let uri = format!("file://{}", self.img.display());
        let replaces = self.id;
        let expire_ms = self.expire_ms;
        let title = self.title.clone();
        let body = self.body.clone();
        let (rt, conn) = self.session()?;
        let id = rt.block_on(async {
            let mut hints: HashMap<&str, Value> = HashMap::new();
            // Normal, not low: GNOME delivers a low-urgency notification
            // without ever showing it as a banner — it only appeared as a
            // dot in the message tray, which is not a bubble.
            hints.insert("urgency", Value::U8(1));
            hints.insert("image-path", Value::from(uri.as_str()));
            hints.insert("category", Value::from("device"));
            hints.insert("desktop-entry", Value::from("podctl"));
            // Not worth keeping in a notification centre: the bubble is
            // a glance at a battery level, and it is gone in seconds.
            hints.insert("transient", Value::Bool(true));
            let reply = conn
                .call_method(
                    Some(SVC),
                    PATH,
                    Some(SVC),
                    "Notify",
                    &(
                        "podctl",
                        replaces,
                        "audio-headphones",
                        title.as_str(),
                        body.as_str(),
                        Vec::<&str>::new(),
                        hints,
                        expire_ms,
                    ),
                )
                .await?;
            let id: u32 = reply.body().deserialize()?;
            Ok::<u32, zbus::Error>(id)
        })?;

        self.id = id;
        self.fp = fp;
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        if self.id != 0 {
            let id = self.id;
            let (rt, conn) = self.session()?;
            let _ = rt.block_on(async {
                conn.call_method(Some(SVC), PATH, Some(SVC), "CloseNotification", &(id,))
                    .await?;
                Ok::<(), zbus::Error>(())
            });
            self.id = 0;
        }
        // The picture stays: the server may still be reading it while
        // the notification fades out.
        Ok(())
    }
}

impl Drop for Notify {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
