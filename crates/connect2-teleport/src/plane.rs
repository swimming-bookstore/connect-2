//! Byte envelope for JSON on Teleport `connect-app`. The payload schema is yours.

use anyhow::{anyhow, Result};
use serde::Serialize;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct ClientMsg {
    pub dst: String,
    pub channel: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct App {
    pub src: String,
    pub dst: String,
    pub data: Vec<u8>,
    pub channel: String,
}

pub fn app(dst: &str, channel: &str, data: Vec<u8>) -> ClientMsg {
    ClientMsg {
        dst: dst.into(),
        channel: channel.into(),
        data,
    }
}

pub async fn send_out<T: Serialize>(
    tx: &mpsc::Sender<ClientMsg>,
    dst: &str,
    channel: &str,
    msg: &T,
) -> Result<()> {
    tx.send(app(dst, channel, serde_json::to_vec(msg)?))
        .await
        .map_err(|_| anyhow!("plane closed"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn send_out_serializes() {
        let (tx, mut rx) = mpsc::channel(1);
        send_out(&tx, "d", "c", &json!({"hello": true}))
            .await
            .unwrap();
        let m = rx.recv().await.unwrap();
        assert_eq!(m.dst, "d");
        assert_eq!(m.channel, "c");
        let v: serde_json::Value = serde_json::from_slice(&m.data).unwrap();
        assert_eq!(v["hello"], true);
    }

    #[test]
    fn app_msg() {
        let m = app("dst-1", "ch-9", b"hi".to_vec());
        assert_eq!(m.dst, "dst-1");
        assert_eq!(m.channel, "ch-9");
        assert_eq!(m.data, b"hi");
    }
}
