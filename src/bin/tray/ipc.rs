use std::time::Duration;

use anyhow::Context;
use tracing::warn;

use podctl::{Request, Response, client};

/// How long opening the menu may wait for a fresh status.
pub const MENU_WAIT: Duration = Duration::from_millis(300);

pub async fn send(req: &Request) -> anyhow::Result<Response> {
    client::async_io::send(req)
        .await
        .with_context(|| format!("talking to podctld at {}", podctl::socket_path().display()))
}

/// Send something the user asked for and tell them when it did not
/// happen; a click that fails in silence looks like a click that missed.
pub fn dispatch(what: &'static str, req: Request) {
    if crate::demo::active() {
        tracing::info!(req = ?req, "demo: {what}");
        return;
    }
    tokio::spawn(async move {
        let why = match send(&req).await {
            Ok(Response::Err(e)) => e.to_string(),
            Err(_) => "podctld is not running".to_string(),
            Ok(_) => return,
        };
        warn!(req = ?req, error = %why, "{what} failed");
        if let Some(n) = crate::notify::get() {
            n.failed(what, &why).await;
        }
    });
}
