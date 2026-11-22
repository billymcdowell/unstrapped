//! History download (docs/PROTOCOL_NOTES.md section 4): GetDataRange, SendHistoricalData, then for every
//! chunk persist raw frames + decoded rows + cursor in ONE SQLite transaction and only after the commit
//! send the ack. Idle watchdog, abort, resume (the strap replays unacked data; rows are deduplicated).

use crate::db::{ChunkStats, Db};
use crate::emitter::{Emitter, emit};
use crate::model::SyncProgress;
use crate::strap::{Inbound, Link};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;
use unstrapped_core::protocol::{DataRange, HistorySync, HostCommand, Packet, ResponseBody, SyncEvent, command_number};

#[derive(Debug, Clone)]
pub struct SyncOptions {
    /// Abort when no history/event/metadata/log frame arrives for this long (NOOP uses 60 s).
    pub idle_timeout: Duration,
    /// How long to wait for the data-range answer before carrying on without it.
    pub request_timeout: Duration,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self { idle_timeout: Duration::from_secs(60), request_timeout: Duration::from_secs(10) }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SyncOutcome {
    pub chunks: usize,
    pub records: usize,
    pub new_frames: usize,
    pub min_ts: Option<i64>,
    pub max_ts: Option<i64>,
    pub completed: bool,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn progress(em: &dyn Emitter, phase: &str, fraction: f64, label: impl Into<String>) {
    emit(em, "sync://progress", &SyncProgress::new(phase, fraction.clamp(0.0, 1.0), label));
}

async fn wait_response(rx: &mut UnboundedReceiver<Inbound>, stash: &mut VecDeque<Inbound>, cmd: u8, family: unstrapped_core::protocol::DeviceFamily, timeout: Duration) -> Option<ResponseBody> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let inb = tokio::time::timeout_at(deadline, rx.recv()).await.ok()??;
        if let Packet::CommandResponse(r) = &inb.packet
            && r.cmd == cmd {
                match r.result {
                    unstrapped_core::protocol::CommandResult::Pending => continue,
                    unstrapped_core::protocol::CommandResult::Success => return r.typed_body(family).ok(),
                    _ => return None,
                }
            }
        // anything else (e.g. history that started early) must not be lost: replay it in the main loop
        if !matches!(inb.packet, Packet::RealtimeHr(_)) {
            stash.push_back(inb);
        }
    }
}

/// Run one history offload on an established link. `rx` must be the sink receiver installed by the caller
/// (every decoded packet of the session arrives on it).
pub async fn run_history_sync(
    link: &Link,
    rx: &mut UnboundedReceiver<Inbound>,
    db: &Db,
    em: &dyn Emitter,
    opts: &SyncOptions,
) -> Result<SyncOutcome, String> {
    let family = link.family;
    progress(em, "requesting", 0.02, "Asking the strap what it has stored…");

    link.send(&HostCommand::GetDataRange).await?;
    let mut stash: VecDeque<Inbound> = VecDeque::new();
    let range = match wait_response(rx, &mut stash, command_number::GET_DATA_RANGE, family, opts.request_timeout).await {
        Some(ResponseBody::DataRange(r)) => Some(r),
        _ => None,
    };
    let span = range.and_then(|r| r.oldest_unix().zip(r.newest_unix())).filter(|(a, b)| b > a);
    let hours = span.map(|(a, b)| ((b - a) as f64 / 3600.0).round().max(1.0) as i64);
    let estimate = range.map(|r: DataRange| r.record_estimate).filter(|e| *e > 0);

    link.send(&HostCommand::SendHistoricalData).await?;
    progress(em, "downloading", 0.03, "Downloading history…");

    let mut hist = HistorySync::new();
    let mut out = SyncOutcome::default();
    let mut last_activity = Instant::now();
    loop {
        let next = match stash.pop_front() {
            Some(i) => Ok(Some(i)),
            None => tokio::time::timeout(Duration::from_millis(500), rx.recv()).await,
        };
        let inb = match next {
            Ok(Some(i)) => i,
            Ok(None) => {
                hist.reset();
                return Err("The connection to the strap was lost during sync. Data already saved is kept; the rest will resume next time.".into());
            }
            Err(_) => {
                if last_activity.elapsed() > opts.idle_timeout {
                    let _ = link.send(&HostCommand::AbortHistoricalTransmits).await;
                    hist.reset();
                    return Err(format!("The strap stopped sending data for {} s; sync aborted and will resume next time.", opts.idle_timeout.as_secs()));
                }
                continue;
            }
        };
        match &inb.packet {
            Packet::Historical(_) | Packet::HistoryMetadata(_) | Packet::Event(_) | Packet::Unknown(_) => last_activity = Instant::now(),
            _ => {}
        }
        match hist.on_packet(&inb.packet, &inb.raw) {
            Some(SyncEvent::Chunk(chunk)) => {
                // 1) persist durably (one transaction) ...
                let db2 = db.clone();
                let strap_id = link.strap_id.clone();
                let channel = inb.characteristic.clone();
                let raw_frames = chunk.raw_frames.clone();
                let end = chunk.end.clone();
                let stats: Result<ChunkStats, String> = tokio::task::spawn_blocking(move || {
                    db2.persist_history_chunk(&strap_id, family, &channel, now_ms(), &raw_frames, &end)
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
                let stats = match stats {
                    Ok(s) => s,
                    Err(e) => {
                        // never ack data we failed to store
                        let _ = link.send(&HostCommand::AbortHistoricalTransmits).await;
                        hist.reset();
                        return Err(format!("Could not save downloaded data ({e}); sync aborted without acknowledging."));
                    }
                };
                // 2) ... and only then acknowledge.
                link.send(&chunk.ack()).await?;
                out.chunks += 1;
                out.records += stats.records;
                out.new_frames += stats.new_frames;
                out.min_ts = match (out.min_ts, stats.min_ts) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                out.max_ts = match (out.max_ts, stats.max_ts) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
                // progress from the data range
                let frac = match (span, out.max_ts) {
                    (Some((a, b)), Some(m)) => ((m / 1000 - a as i64) as f64 / (b - a) as f64).clamp(0.0, 0.99),
                    _ => match estimate {
                        Some(e) => (out.records as f64 / e as f64).clamp(0.0, 0.99),
                        None => (out.chunks as f64 / (out.chunks as f64 + 20.0)).min(0.9),
                    },
                };
                let label = match hours {
                    Some(h) => format!("Downloading {h} h of data… {}%", (frac * 100.0).round() as i64),
                    None => format!("Downloading history… {} readings", out.records),
                };
                progress(em, "downloading", frac, label);
            }
            Some(SyncEvent::Completed) => {
                out.completed = true;
                break;
            }
            Some(SyncEvent::Started) | None => {}
        }
    }
    Ok(out)
}
