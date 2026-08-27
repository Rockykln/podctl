use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use zbus::zvariant::Value;

use super::{Backend, Frame};
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
        }
    }

    fn rt() -> Result<tokio::runtime::Runtime> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("tokio runtime")
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
        let id = Self::rt()?.block_on(async {
            let conn = zbus::Connection::session().await?;
            let mut hints: HashMap<&str, Value> = HashMap::new();
            hints.insert("urgency", Value::U8(0));
            hints.insert("image-path", Value::from(uri.as_str()));
            hints.insert("category", Value::from("device"));
            hints.insert("desktop-entry", Value::from("podctl"));
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
                        "AirPods",
                        "",
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
            let _ = Self::rt()?.block_on(async {
                let conn = zbus::Connection::session().await?;
                conn.call_method(Some(SVC), PATH, Some(SVC), "CloseNotification", &(id,))
                    .await?;
                Ok::<(), zbus::Error>(())
            });
            self.id = 0;
        }
        let _ = std::fs::remove_file(&self.img);
        Ok(())
    }
}

impl Drop for Notify {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn fingerprint(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
