//! TCP emulator service: one emulated strap, one host connection at a time (like BLE).

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::strap::{EmulatedStrap, StrapOptions};
use crate::wire::{EmuMsg, HostMsg};
use crate::EmulatorConfig;

/// Server-level options (use with [`serve_with`]); [`serve`] uses the defaults.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// Stream history as fast as the host acknowledges instead of at a BLE-like ~150 records/s.
    pub fast_history: bool,
    /// How often the strap is ticked while a host is connected.
    pub tick_ms: u64,
    /// Strap-level options (record interval, MTU, chunk size, timezone).
    pub strap: StrapOptions,
}

impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions { fast_history: false, tick_ms: 50, strap: StrapOptions::default() }
    }
}

struct Shared {
    strap: EmulatedStrap,
    start_ms: i64,
    t0: Instant,
    skew_ms: i64,
    scale: f64,
    in_range: bool,
    busy: bool,
}

impl Shared {
    fn now(&self) -> i64 {
        self.start_ms + (self.t0.elapsed().as_secs_f64() * 1000.0 * self.scale) as i64 + self.skew_ms
    }
}

/// Listen on `addr` and serve one emulated strap (one host connection at a time, like BLE).
pub async fn serve(addr: &str, config: EmulatorConfig) -> std::io::Result<()> {
    serve_with(addr, config, ServerOptions::default()).await
}

/// [`serve`] with explicit [`ServerOptions`].
pub async fn serve_with(addr: &str, config: EmulatorConfig, opts: ServerOptions) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    eprintln!("[emulator] listening on {}", listener.local_addr()?);
    serve_on(listener, config, opts).await
}

/// Serve on an already-bound listener (handy with port 0 in tests). Runs until the task is dropped.
pub async fn serve_on(listener: TcpListener, config: EmulatorConfig, opts: ServerOptions) -> std::io::Result<()> {
    let mut strap_opts = opts.strap.clone();
    if opts.fast_history {
        strap_opts.history_records_per_sec = None;
    }
    let scale = if config.time_scale > 0.0 { config.time_scale } else { 1.0 };
    let strap = EmulatedStrap::with_options(config, strap_opts);
    let start_ms = strap.now_ms();
    eprintln!(
        "[emulator] {:?} '{}' id={} time_scale={} start={} history_days<=14 fast_history={}",
        strap.family(),
        strap.device_name(),
        strap.device_id(),
        scale,
        start_ms,
        opts.fast_history
    );
    let shared = Arc::new(Mutex::new(Shared { strap, start_ms, t0: Instant::now(), skew_ms: 0, scale, in_range: true, busy: false }));
    let tick = Duration::from_millis(opts.tick_ms.max(1));
    loop {
        let (stream, peer) = listener.accept().await?;
        let sh = shared.clone();
        tokio::spawn(async move {
            let (in_range, busy) = {
                let mut g = sh.lock().await;
                let r = (g.in_range, g.busy);
                if g.in_range && !g.busy {
                    g.busy = true;
                }
                r
            };
            if !in_range {
                eprintln!("[emulator] {peer}: out of range (control-only)");
                let _ = control_only(stream, sh).await;
            } else if busy {
                eprintln!("[emulator] {peer}: refused (already connected)");
                let _ = refuse(stream, sh).await;
            } else {
                eprintln!("[emulator] {peer}: host connected");
                let r = session(stream, sh.clone(), tick).await;
                let mut g = sh.lock().await;
                g.busy = false;
                g.strap.on_disconnect();
                eprintln!("[emulator] {peer}: host gone ({})", r.map(|_| "closed".to_string()).unwrap_or_else(|e| e.to_string()));
            }
        });
    }
}

fn line(m: &EmuMsg) -> String {
    let mut s = serde_json::to_string(m).expect("serializable");
    s.push('\n');
    s
}

async fn adv(sh: &Arc<Mutex<Shared>>) -> EmuMsg {
    let g = sh.lock().await;
    EmuMsg::Advertisement {
        name: g.strap.device_name().to_string(),
        device_id: g.strap.device_id().to_string(),
        family: g.strap.family(),
        service_uuid: g.strap.family().service_uuid().to_string(),
        rssi: -52,
        in_range: g.in_range,
    }
}

async fn refuse(mut stream: TcpStream, sh: Arc<Mutex<Shared>>) -> std::io::Result<()> {
    let _ = sh;
    stream.write_all(line(&EmuMsg::Error { message: "strap is already connected to another host".into() }).as_bytes()).await?;
    Ok(())
}

/// While out of range the strap does not accept GATT connections, but dev controls still work so a
/// tool can say `in_range`.
async fn control_only(stream: TcpStream, sh: Arc<Mutex<Shared>>) -> std::io::Result<()> {
    let (rd, mut wr) = stream.into_split();
    let a = adv(&sh).await;
    wr.write_all(line(&a).as_bytes()).await?;
    let mut lines = BufReader::new(rd).lines();
    while let Some(l) = lines.next_line().await? {
        match serde_json::from_str::<HostMsg>(&l) {
            Ok(HostMsg::Control { action, value }) => {
                let mut g = sh.lock().await;
                let mut out = Vec::new();
                control(&mut g, &action, value, &mut out);
                for m in out {
                    wr.write_all(line(&m).as_bytes()).await?;
                }
            }
            _ => {
                wr.write_all(line(&EmuMsg::Error { message: "out of range".into() }).as_bytes()).await?;
            }
        }
    }
    Ok(())
}

/// Returns true when the session must end (host dropped by a control).
fn control(g: &mut Shared, action: &str, value: Option<f64>, out: &mut Vec<EmuMsg>) -> bool {
    let now = g.now();
    eprintln!("[emulator] control {action} {value:?}");
    match action {
        "out_of_range" => {
            g.in_range = false;
            out.push(EmuMsg::Disconnected { reason: "out_of_range".into() });
            return true;
        }
        "in_range" => g.in_range = true,
        "take_off" => g.strap.take_off(now),
        "put_on" => g.strap.put_on(now),
        "advance_minutes" => g.skew_ms += (value.unwrap_or(0.0) * 60_000.0) as i64,
        "set_battery" => g.strap.set_battery(value.unwrap_or(50.0), now),
        "charge_on" => g.strap.set_charging(true, now),
        "charge_off" => g.strap.set_charging(false, now),
        other => out.push(EmuMsg::Error { message: format!("unknown control '{other}'") }),
    }
    false
}

async fn session(stream: TcpStream, sh: Arc<Mutex<Shared>>, tick: Duration) -> std::io::Result<()> {
    let (rd, mut wr) = stream.into_split();
    let a = adv(&sh).await;
    wr.write_all(line(&a).as_bytes()).await?;
    let mut lines = BufReader::new(rd).lines();
    let mut ticker = tokio::time::interval(tick);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut gatt = false;
    loop {
        let mut out: Vec<EmuMsg> = Vec::new();
        let mut end = false;
        tokio::select! {
            l = lines.next_line() => {
                let Some(l) = l? else { return Ok(()) };
                if l.trim().is_empty() { continue; }
                let mut g = sh.lock().await;
                match serde_json::from_str::<HostMsg>(&l) {
                    Err(e) => out.push(EmuMsg::Error { message: format!("bad message: {e}") }),
                    Ok(HostMsg::Connect) => {
                        let now = g.now();
                        g.strap.on_connect(now);
                        gatt = true;
                        out.push(EmuMsg::Connected { characteristics: g.strap.characteristics() });
                    }
                    Ok(HostMsg::Subscribe { characteristic }) => {
                        if gatt { g.strap.set_subscribed(&characteristic, true); } else { out.push(EmuMsg::Error { message: "not connected".into() }); }
                    }
                    Ok(HostMsg::Unsubscribe { characteristic }) => g.strap.set_subscribed(&characteristic, false),
                    Ok(HostMsg::Write { characteristic, hex }) => {
                        match hex::decode(&hex) {
                            _ if !gatt => out.push(EmuMsg::Error { message: "not connected".into() }),
                            Ok(b) => { let now = g.now(); g.strap.on_write(&characteristic, &b, now); }
                            Err(e) => out.push(EmuMsg::Error { message: format!("bad hex: {e}") }),
                        }
                    }
                    Ok(HostMsg::Disconnect) => {
                        g.strap.on_disconnect();
                        gatt = false;
                        out.push(EmuMsg::Disconnected { reason: "requested".into() });
                    }
                    Ok(HostMsg::Control { action, value }) => { end = control(&mut g, &action, value, &mut out); }
                }
                if gatt && !end {
                    let now = g.now();
                    flush(&mut g, now, &mut out);
                }
            }
            _ = ticker.tick() => {
                if gatt {
                    let mut g = sh.lock().await;
                    let now = g.now();
                    flush(&mut g, now, &mut out);
                }
            }
        }
        if !out.is_empty() {
            let mut buf = String::new();
            for m in &out {
                buf.push_str(&line(m));
            }
            wr.write_all(buf.as_bytes()).await?;
        }
        if end {
            return Ok(());
        }
    }
}

fn flush(g: &mut Shared, now: i64, out: &mut Vec<EmuMsg>) {
    for n in g.strap.tick(now) {
        out.push(EmuMsg::Notify { characteristic: n.characteristic, hex: hex::encode(n.bytes) });
    }
}
