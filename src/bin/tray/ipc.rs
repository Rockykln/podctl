use anyhow::Context;

use podctl::{Request, Response, client};

pub async fn send(req: &Request) -> anyhow::Result<Response> {
    client::async_io::send(req)
        .await
        .with_context(|| format!("talking to podctld at {}", podctl::socket_path().display()))
}
