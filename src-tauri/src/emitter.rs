//! Event sink abstraction so the sync / strap / compute code can run without a Tauri window (tests).

use serde::Serialize;
use serde_json::Value;
use std::sync::Mutex;

pub trait Emitter: Send + Sync + 'static {
    fn emit_json(&self, event: &str, payload: Value);
}

pub fn emit<T: Serialize>(e: &dyn Emitter, event: &str, payload: &T) {
    if let Ok(v) = serde_json::to_value(payload) {
        e.emit_json(event, v);
    }
}

/// Emits to every webview window via Tauri.
pub struct TauriEmitter(pub tauri::AppHandle);

impl Emitter for TauriEmitter {
    fn emit_json(&self, event: &str, payload: Value) {
        use tauri::Emitter as _;
        let _ = self.0.emit(event, payload);
    }
}

/// Drops everything.
pub struct NullEmitter;

impl Emitter for NullEmitter {
    fn emit_json(&self, _event: &str, _payload: Value) {}
}

/// Records everything (tests).
#[derive(Default)]
pub struct CollectEmitter {
    pub events: Mutex<Vec<(String, Value)>>,
}

impl Emitter for CollectEmitter {
    fn emit_json(&self, event: &str, payload: Value) {
        self.events.lock().unwrap().push((event.to_string(), payload));
    }
}

impl CollectEmitter {
    pub fn named(&self, name: &str) -> Vec<Value> {
        self.events.lock().unwrap().iter().filter(|(n, _)| n == name).map(|(_, v)| v.clone()).collect()
    }
}
