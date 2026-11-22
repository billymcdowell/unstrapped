//! `StrapTransport`: how bytes get to and from a strap. `BleTransport` talks to a real strap over
//! Bluetooth LE (btleplug); `EmulatorTransport` talks to the `unstrapped-emulator` TCP service.
//! The connection manager and sync code are written against the trait only.

use async_trait::async_trait;
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;
use unstrapped_core::protocol::DeviceFamily;

pub type TResult<T> = Result<T, String>;

#[derive(Debug, Clone)]
pub struct ConnectInfo {
    pub name: String,
    /// Stable identifier the app remembers (CoreBluetooth UUID for BLE, device id for the emulator).
    pub device_id: String,
    pub family: DeviceFamily,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnPhase {
    Scanning,
    Connecting,
}

#[derive(Debug, Clone)]
pub enum TransportEvent {
    Notify { characteristic: String, bytes: Vec<u8> },
    Disconnected(String),
}

#[async_trait]
pub trait StrapTransport: Send {
    /// Scan (if needed), connect, discover and subscribe to every notify characteristic.
    async fn connect(&mut self, phase: &(dyn Fn(ConnPhase) + Send + Sync)) -> TResult<ConnectInfo>;
    /// Notifications and disconnects. Call once after `connect`.
    fn take_events(&mut self) -> Option<mpsc::Receiver<TransportEvent>>;
    /// Write a command frame (write-with-response) to a characteristic.
    async fn write(&mut self, characteristic: &str, bytes: &[u8]) -> TResult<()>;
    async fn disconnect(&mut self);
    /// Developer controls; only the emulator implements these.
    async fn control(&mut self, _action: &str, _value: Option<f64>) -> TResult<()> {
        Err("This connection has no developer controls (emulator only).".into())
    }
    fn kind(&self) -> &'static str;
}

// ===================================================================================== BLE

pub struct BleTransport {
    remembered_id: Option<String>,
    scan_timeout: Duration,
    peripheral: Option<btleplug::platform::Peripheral>,
    chars: Vec<btleplug::api::Characteristic>,
    events: Option<mpsc::Receiver<TransportEvent>>,
    forwarder: Option<tokio::task::JoinHandle<()>>,
}

impl BleTransport {
    pub fn new(remembered_id: Option<String>, scan_timeout: Duration) -> Self {
        Self { remembered_id, scan_timeout, peripheral: None, chars: vec![], events: None, forwarder: None }
    }
}

fn berr<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn family_of_services(services: impl IntoIterator<Item = uuid::Uuid>) -> Option<DeviceFamily> {
    services.into_iter().find_map(|u| DeviceFamily::from_service_uuid(&u.to_string()))
}

#[async_trait]
impl StrapTransport for BleTransport {
    fn kind(&self) -> &'static str {
        "ble"
    }

    async fn connect(&mut self, phase: &(dyn Fn(ConnPhase) + Send + Sync)) -> TResult<ConnectInfo> {
        use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter, WriteType, RetrievePeripheralsOptions};
        use btleplug::platform::{Manager, PeripheralId};

        let manager = Manager::new().await.map_err(|e| format!("Bluetooth is not available: {e}"))?;
        let adapter = manager.adapters().await.map_err(berr)?.into_iter().next().ok_or("No Bluetooth adapter found. Turn Bluetooth on and allow the app in System Settings > Privacy & Security > Bluetooth.")?;

        phase(ConnPhase::Scanning);
        let mut found: Option<btleplug::platform::Peripheral> = None;

        // 1. Remembered strap: ask CoreBluetooth for it directly, no scan needed.
        if let Some(id) = self.remembered_id.as_deref().and_then(|s| uuid::Uuid::parse_str(s).ok()) {
            let opts = RetrievePeripheralsOptions { identifiers: Some(vec![PeripheralId::from(id)]), services: None };
            if let Ok(list) = adapter.retrieve_peripherals(opts).await {
                found = list.into_iter().next();
            }
        }

        // 2. Scan for either family's service.
        if found.is_none() {
            let services = vec![
                uuid::Uuid::parse_str(DeviceFamily::Whoop4.service_uuid()).unwrap(),
                uuid::Uuid::parse_str(DeviceFamily::Whoop5.service_uuid()).unwrap(),
            ];
            let mut events = adapter.events().await.map_err(berr)?;
            adapter.start_scan(ScanFilter { services }).await.map_err(berr)?;
            let deadline = tokio::time::Instant::now() + self.scan_timeout;
            let mut other: Option<btleplug::platform::Peripheral> = None;
            loop {
                let ev = tokio::time::timeout_at(deadline, events.next()).await;
                let Ok(Some(ev)) = ev else { break };
                let id = match ev {
                    CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => id,
                    CentralEvent::ServicesAdvertisement { id, .. } => id,
                    _ => continue,
                };
                let Ok(p) = adapter.peripheral(&id).await else { continue };
                let Ok(Some(props)) = p.properties().await else { continue };
                if family_of_services(props.services.iter().copied()).is_none() {
                    continue;
                }
                match &self.remembered_id {
                    Some(r) if *r != id.to_string() => {
                        other.get_or_insert(p);
                    }
                    _ => {
                        found = Some(p);
                        break;
                    }
                }
            }
            let _ = adapter.stop_scan().await;
            if found.is_none() {
                found = other;
            }
        }
        let p = found.ok_or("No WHOOP strap found nearby. Make sure it is charged, close to the Mac, and not connected to the WHOOP app.")?;

        phase(ConnPhase::Connecting);
        if !p.is_connected().await.unwrap_or(false) {
            p.connect_with_timeout(Duration::from_secs(20)).await.map_err(|e| format!("Could not connect to the strap: {e}"))?;
        }
        p.discover_services_with_timeout(Duration::from_secs(20)).await.map_err(berr)?;
        let services: Vec<uuid::Uuid> = p.services().iter().map(|s| s.uuid).collect();
        let family = family_of_services(services).ok_or("Connected device does not expose a WHOOP service.")?;
        let name = p.properties().await.ok().flatten().and_then(|pr| pr.local_name.or(pr.advertisement_name)).unwrap_or_else(|| "WHOOP strap".into());
        let device_id = p.id().to_string();

        let all = p.characteristics();
        let mut chars = vec![];
        for c in family.notify_characteristics() {
            if let Some(ch) = all.iter().find(|x| x.uuid.to_string() == c.uuid) {
                p.subscribe(ch).await.map_err(|e| format!("Could not subscribe to {}: {e}", c.uuid))?;
                chars.push(ch.clone());
            }
        }
        // Standard heart rate (works unbonded); best effort.
        if let Some(hr) = all.iter().find(|x| x.uuid.to_string() == unstrapped_core::protocol::gatt::STANDARD_HEART_RATE_MEASUREMENT) {
            let _ = p.subscribe(hr).await;
        }
        if let Some(cmd) = all.iter().find(|x| x.uuid.to_string() == family.command_characteristic_uuid()) {
            chars.push(cmd.clone());
        } else {
            return Err("The strap has no command characteristic.".into());
        }

        // Forward notifications and disconnects.
        let (tx, rx) = mpsc::channel(4096);
        let mut notifs = p.notifications().await.map_err(berr)?;
        let mut central_events = adapter.events().await.map_err(berr)?;
        let my_id = p.id();
        self.forwarder = Some(tokio::spawn(async move {
            loop {
                tokio::select! {
                    n = notifs.next() => match n {
                        Some(n) => { if tx.send(TransportEvent::Notify { characteristic: n.uuid.to_string(), bytes: n.value }).await.is_err() { break; } }
                        None => { let _ = tx.send(TransportEvent::Disconnected("notification stream ended".into())).await; break; }
                    },
                    ev = central_events.next() => match ev {
                        Some(CentralEvent::DeviceDisconnected(id)) if id == my_id => {
                            let _ = tx.send(TransportEvent::Disconnected("strap disconnected".into())).await; break;
                        }
                        Some(_) => {}
                        None => {}
                    },
                }
            }
        }));
        let _ = WriteType::WithResponse; // (imported for write below)
        self.chars = chars;
        self.peripheral = Some(p);
        self.events = Some(rx);
        Ok(ConnectInfo { name, device_id, family })
    }

    fn take_events(&mut self) -> Option<mpsc::Receiver<TransportEvent>> {
        self.events.take()
    }

    async fn write(&mut self, characteristic: &str, bytes: &[u8]) -> TResult<()> {
        use btleplug::api::{Peripheral as _, WriteType};
        let p = self.peripheral.as_ref().ok_or("not connected")?;
        let ch = self.chars.iter().find(|c| c.uuid.to_string() == characteristic.to_ascii_lowercase()).ok_or("unknown characteristic")?;
        p.write(ch, bytes, WriteType::WithResponse).await.map_err(berr)
    }

    async fn disconnect(&mut self) {
        use btleplug::api::Peripheral as _;
        if let Some(f) = self.forwarder.take() {
            f.abort();
        }
        if let Some(p) = self.peripheral.take() {
            let _ = p.disconnect().await;
        }
    }
}

// ===================================================================================== emulator

pub struct EmulatorTransport {
    address: String,
    client: Option<unstrapped_emulator::client::EmulatorClient>,
    command_char: String,
    events: Option<mpsc::Receiver<TransportEvent>>,
    forwarder: Option<tokio::task::JoinHandle<()>>,
}

impl EmulatorTransport {
    pub fn new(address: String) -> Self {
        Self { address, client: None, command_char: String::new(), events: None, forwarder: None }
    }
}

#[async_trait]
impl StrapTransport for EmulatorTransport {
    fn kind(&self) -> &'static str {
        "emulator"
    }

    async fn connect(&mut self, phase: &(dyn Fn(ConnPhase) + Send + Sync)) -> TResult<ConnectInfo> {
        use unstrapped_emulator::wire::EmuMsg;
        phase(ConnPhase::Scanning);
        let (mut client, adv) = unstrapped_emulator::client::EmulatorClient::connect(&self.address)
            .await
            .map_err(|e| format!("Could not reach the strap emulator at {} ({e}). Start it with `cargo run -p unstrapped-emulator`.", self.address))?;
        let EmuMsg::Advertisement { name, device_id, family, in_range, .. } = adv else {
            return Err("Emulator sent an unexpected first message.".into());
        };
        if !in_range {
            return Err("The emulated strap is out of range.".into());
        }
        phase(ConnPhase::Connecting);
        let chars = client.gatt_connect().await.map_err(|e| format!("Emulator connect failed: {e}"))?;
        for c in chars.iter().filter(|c| c.notify) {
            client.subscribe(&c.uuid).await.map_err(|e| format!("Emulator subscribe failed: {e}"))?;
        }
        self.command_char = family.command_characteristic_uuid().to_string();
        let mut raw_events = client.take_events();
        let (tx, rx) = mpsc::channel(4096);
        self.forwarder = Some(tokio::spawn(async move {
            while let Some(m) = raw_events.recv().await {
                match m {
                    EmuMsg::Notify { characteristic, hex } => {
                        if let Ok(bytes) = hex::decode(hex)
                            && tx.send(TransportEvent::Notify { characteristic, bytes }).await.is_err() {
                                return;
                            }
                    }
                    EmuMsg::Disconnected { reason } => {
                        let _ = tx.send(TransportEvent::Disconnected(reason)).await;
                        return;
                    }
                    _ => {}
                }
            }
            let _ = tx.send(TransportEvent::Disconnected("emulator connection closed".into())).await;
        }));
        self.client = Some(client);
        self.events = Some(rx);
        Ok(ConnectInfo { name, device_id, family })
    }

    fn take_events(&mut self) -> Option<mpsc::Receiver<TransportEvent>> {
        self.events.take()
    }

    async fn write(&mut self, characteristic: &str, bytes: &[u8]) -> TResult<()> {
        let c = self.client.as_mut().ok_or("not connected")?;
        let ch = if characteristic.is_empty() { self.command_char.as_str() } else { characteristic };
        c.write(ch, bytes).await.map_err(|e| e.to_string())
    }

    async fn disconnect(&mut self) {
        if let Some(c) = self.client.as_mut() {
            let _ = c.disconnect().await;
        }
        self.client = None;
        if let Some(f) = self.forwarder.take() {
            f.abort();
        }
    }

    async fn control(&mut self, action: &str, value: Option<f64>) -> TResult<()> {
        let c = self.client.as_mut().ok_or("not connected")?;
        c.control(action, value).await.map_err(|e| e.to_string())
    }
}

// ===================================================================================== tests

/// In-process transport: drives a `unstrapped_emulator::EmulatedStrap` directly (no sockets), ticking it on a
/// background task with the real clock.
#[cfg(test)]
pub mod inproc {
    use super::*;
    use std::sync::{Arc, Mutex};
    use unstrapped_emulator::EmulatedStrap;

    pub struct InProcTransport {
        strap: Arc<Mutex<EmulatedStrap>>,
        events: Option<mpsc::Receiver<TransportEvent>>,
        ticker: Option<tokio::task::JoinHandle<()>>,
        pub fixed_now_ms: Option<i64>,
    }

    pub fn now_ms() -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
    }

    impl InProcTransport {
        pub fn new(strap: EmulatedStrap) -> Self {
            Self { strap: Arc::new(Mutex::new(strap)), events: None, ticker: None, fixed_now_ms: None }
        }
    }

    #[async_trait]
    impl StrapTransport for InProcTransport {
        fn kind(&self) -> &'static str {
            "emulator"
        }
        async fn connect(&mut self, _phase: &(dyn Fn(ConnPhase) + Send + Sync)) -> TResult<ConnectInfo> {
            let (family, name, id, chars) = {
                let mut s = self.strap.lock().unwrap();
                s.on_connect(now_ms());
                let chars = s.characteristics();
                for c in chars.iter().filter(|c| c.notify) {
                    s.set_subscribed(&c.uuid, true);
                }
                (s.family(), s.device_name().to_string(), s.device_id().to_string(), chars)
            };
            let _ = chars;
            let (tx, rx) = mpsc::channel(65536);
            let strap = self.strap.clone();
            self.ticker = Some(tokio::spawn(async move {
                loop {
                    let notes = strap.lock().unwrap().tick(now_ms());
                    for n in notes {
                        if tx.send(TransportEvent::Notify { characteristic: n.characteristic, bytes: n.bytes }).await.is_err() {
                            return;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            }));
            self.events = Some(rx);
            Ok(ConnectInfo { name, device_id: id, family })
        }
        fn take_events(&mut self) -> Option<mpsc::Receiver<TransportEvent>> {
            self.events.take()
        }
        async fn write(&mut self, characteristic: &str, bytes: &[u8]) -> TResult<()> {
            self.strap.lock().unwrap().on_write(characteristic, bytes, now_ms());
            Ok(())
        }
        async fn disconnect(&mut self) {
            if let Some(t) = self.ticker.take() {
                t.abort();
            }
        }
    }
}
