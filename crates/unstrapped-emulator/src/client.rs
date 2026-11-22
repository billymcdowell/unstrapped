//! Async client for the emulator service — the app's "simulated strap" transport. SHARED CONTRACT.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::mpsc;

use crate::wire::{EmuMsg, HostMsg};
use crate::CharacteristicInfo;

pub struct EmulatorClient {
    writer: OwnedWriteHalf,
    events: Option<mpsc::Receiver<EmuMsg>>,
    connected: mpsc::UnboundedReceiver<Vec<CharacteristicInfo>>,
}

fn io_err(kind: std::io::ErrorKind, m: impl Into<String>) -> std::io::Error {
    std::io::Error::new(kind, m.into())
}

impl EmulatorClient {
    /// Open the TCP connection and read the `Advertisement` (returned alongside the client).
    /// Errors if nothing is listening or the emulator reports `in_range: false`.
    pub async fn connect(addr: &str) -> std::io::Result<(EmulatorClient, EmuMsg)> {
        let (c, adv) = Self::connect_any(addr).await?;
        if let EmuMsg::Advertisement { in_range: false, .. } = adv {
            return Err(io_err(std::io::ErrorKind::NotConnected, "strap is out of range"));
        }
        if let EmuMsg::Error { message } = &adv {
            return Err(io_err(std::io::ErrorKind::ConnectionRefused, message.clone()));
        }
        Ok((c, adv))
    }

    /// Like [`connect`](Self::connect) but also succeeds when the strap is out of range, so a dev
    /// tool can still send controls such as `in_range`.
    pub async fn connect_any(addr: &str) -> std::io::Result<(EmulatorClient, EmuMsg)> {
        let stream = TcpStream::connect(addr).await?;
        let (rd, writer) = stream.into_split();
        let mut lines = BufReader::new(rd).lines();
        let first = lines
            .next_line()
            .await?
            .ok_or_else(|| io_err(std::io::ErrorKind::UnexpectedEof, "emulator closed before advertising"))?;
        let adv: EmuMsg = serde_json::from_str(&first).map_err(|e| io_err(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let (tx, rx) = mpsc::channel::<EmuMsg>(8192);
        let (ctx, crx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                match lines.next_line().await {
                    Ok(Some(l)) => match serde_json::from_str::<EmuMsg>(&l) {
                        Ok(EmuMsg::Connected { characteristics }) => {
                            let _ = ctx.send(characteristics);
                        }
                        Ok(m) => {
                            if tx.send(m).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(EmuMsg::Error { message: format!("bad emulator message: {e}") }).await;
                        }
                    },
                    _ => {
                        let _ = tx.send(EmuMsg::Disconnected { reason: "emulator connection closed".into() }).await;
                        break;
                    }
                }
            }
        });
        Ok((EmulatorClient { writer, events: Some(rx), connected: crx }, adv))
    }

    async fn send(&mut self, m: &HostMsg) -> std::io::Result<()> {
        let mut s = serde_json::to_string(m).map_err(|e| io_err(std::io::ErrorKind::InvalidData, e.to_string()))?;
        s.push('\n');
        self.writer.write_all(s.as_bytes()).await
    }

    /// Send `Connect` and wait for `Connected`; returns the characteristic list.
    pub async fn gatt_connect(&mut self) -> std::io::Result<Vec<CharacteristicInfo>> {
        self.send(&HostMsg::Connect).await?;
        match tokio::time::timeout(Duration::from_secs(5), self.connected.recv()).await {
            Ok(Some(c)) => Ok(c),
            Ok(None) => Err(io_err(std::io::ErrorKind::ConnectionAborted, "emulator closed during connect")),
            Err(_) => Err(io_err(std::io::ErrorKind::TimedOut, "no Connected reply from emulator")),
        }
    }

    pub async fn subscribe(&mut self, characteristic: &str) -> std::io::Result<()> {
        self.send(&HostMsg::Subscribe { characteristic: characteristic.to_string() }).await
    }

    pub async fn write(&mut self, characteristic: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.send(&HostMsg::Write { characteristic: characteristic.to_string(), hex: hex::encode(bytes) }).await
    }

    pub async fn control(&mut self, action: &str, value: Option<f64>) -> std::io::Result<()> {
        self.send(&HostMsg::Control { action: action.to_string(), value }).await
    }

    pub async fn disconnect(&mut self) -> std::io::Result<()> {
        self.send(&HostMsg::Disconnect).await
    }

    /// Take the stream of incoming messages (`Notify`, `Disconnected`, `Error`). Call once.
    pub fn take_events(&mut self) -> mpsc::Receiver<EmuMsg> {
        self.events.take().unwrap_or_else(|| mpsc::channel(1).1)
    }
}
