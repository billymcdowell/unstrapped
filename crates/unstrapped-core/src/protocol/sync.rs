//! History synchronisation: what the host and the strap say to each other, and a small sans-IO
//! helper that enforces **persist before acknowledge**.
//!
//! # Sequence (WHOOP 4; WHOOP 5 differences marked)
//!
//! ```text
//!  host                                           strap
//!  ----                                           -----
//!  (WHOOP 5 only) write CLIENT_HELLO  ----------->      DeviceFamily::client_hello()
//!  subscribe to every notify characteristic
//!  GetBatteryLevel (confirmed write) ------------>      completes pairing/bond on WHOOP 4
//!  (optional) GetHello, GetClock, SetClock ------>      read the clock back; do not trust the write ack
//!  GetDataRange --------------------------------->
//!                                       <---------      CommandResponse Pending (WHOOP 5: empty body)
//!                                       <---------      CommandResponse Success + DataRange body
//!  (WHOOP 5 requires the range request first; WHOOP 4 may skip it)
//!  SendHistoricalData ---------------------------->
//!                                       <---------      CommandResponse (acceptance only, not data)
//!                                       <---------      HistoryMetadata::Start
//!                                       <---------      Historical record  x N   (type 47)
//!                                       <---------      HistoryMetadata::End { ack_token, .. }   (chunk boundary)
//!  1. decode the chunk's records
//!  2. persist records + raw frames + cursor (durably)
//!  3. only then:
//!  HistoricalDataResult { ack_token } ------------>     verbatim token; strap may now reclaim storage
//!                                       <---------      more records ... another End ... (repeat)
//!                                       <---------      HistoryMetadata::Complete
//! ```
//!
//! Rules that matter:
//!
//! * **Persist before ack.** The ack lets the strap free the chunk. Acknowledging first and then
//!   crashing loses data permanently. If persistence fails, do **not** ack; abort or reconnect.
//! * **Echo the token verbatim.** Never rebuild it from timestamps; its second word is wrap state.
//! * One `Start` can be followed by many `End`s (a chunk roughly every 50 records in high-rate
//!   offload). An `End` with no records is still acknowledged: that is how the offload advances.
//! * `Complete` closes the attempt; it does not commit an open chunk and does not prove everything
//!   was delivered. Compare with [`DataRange`](super::DataRange) afterwards.
//! * Unacknowledged data is replayed after a reconnect, so duplicates are normal: key stored
//!   samples by timestamp.
//! * [`HostCommand::AbortHistoricalTransmits`](super::HostCommand) stops the stream without trimming.
//!   Tear local state down regardless of whether the strap answers.
//! * Guard with an idle watchdog (NOOP uses 60 s without any type 47/48/49/50 frame) and resume from
//!   persisted state next time.
//! * Do not re-run the connect handshake in the middle of an offload (observed to stop streaming).

use super::command::HostCommand;
use super::frame::Frame;
use super::history::{HistoricalPacket, HistoryEnd, HistoryMetadata, HistoryRecord};
use super::packet::Packet;

/// One acknowledgement-bounded chunk of history, handed to the application to persist.
///
/// [`ack`](Self::ack) consumes the chunk: the type system makes you hold the chunk until you have
/// stored it, and then spend it exactly once.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingChunk {
    /// Decoded records, in arrival order.
    pub records: Vec<HistoryRecord>,
    /// Type-47 frames this crate could not map (archive them).
    pub unmapped: Vec<Frame>,
    /// Every raw frame of the chunk (type 47 frames, in arrival order) for the raw archive.
    pub raw_frames: Vec<Vec<u8>>,
    /// The `HISTORY_END` that closed the chunk.
    pub end: HistoryEnd,
}

impl PendingChunk {
    /// The acknowledgement command for this chunk. Call **only after** the chunk is durably stored.
    pub fn ack(self) -> HostCommand {
        HostCommand::HistoricalDataResult { ack_token: self.end.ack_token }
    }
}

/// What a received packet means for the offload.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncEvent {
    /// `HISTORY_START` seen; a new offload stream is open.
    Started,
    /// A chunk closed; persist it, then send [`PendingChunk::ack`].
    Chunk(PendingChunk),
    /// `HISTORY_COMPLETE` seen; the stream is over.
    Completed,
}

/// Collects type-47 records between metadata markers. Feed it every packet (with its raw bytes) in
/// arrival order.
#[derive(Debug, Default)]
pub struct HistorySync {
    records: Vec<HistoryRecord>,
    unmapped: Vec<Frame>,
    raw_frames: Vec<Vec<u8>>,
    active: bool,
}

impl HistorySync {
    /// Fresh session state.
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` between `Start` and `Complete`.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Discard any partially collected chunk (disconnect / abort). Nothing was acknowledged, so the
    /// strap will replay it.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Process one packet. `raw` is its on-wire frame (kept for the raw archive).
    pub fn on_packet(&mut self, packet: &Packet, raw: &[u8]) -> Option<SyncEvent> {
        match packet {
            Packet::HistoryMetadata(HistoryMetadata::Start { .. }) => {
                self.reset();
                self.active = true;
                Some(SyncEvent::Started)
            }
            Packet::Historical(h) => {
                self.raw_frames.push(raw.to_vec());
                match h {
                    HistoricalPacket::Record(r) => self.records.push(r.clone()),
                    HistoricalPacket::Unmapped { frame, .. } => self.unmapped.push(frame.clone()),
                }
                None
            }
            Packet::HistoryMetadata(HistoryMetadata::End { end, .. }) => {
                // High-rate offload sends one Start then repeated Ends, so stay active.
                self.active = true;
                Some(SyncEvent::Chunk(PendingChunk {
                    records: std::mem::take(&mut self.records),
                    unmapped: std::mem::take(&mut self.unmapped),
                    raw_frames: std::mem::take(&mut self.raw_frames),
                    end: end.clone(),
                }))
            }
            Packet::HistoryMetadata(HistoryMetadata::Complete { .. }) => {
                self.active = false;
                Some(SyncEvent::Completed)
            }
            _ => None,
        }
    }
}
