import { useEffect, useRef, useState } from "react";
import * as api from "../api/client";
import type { ExportKind, Settings, Transport } from "../api/types";
import { Card, EmptyState, Segmented, StatTile } from "../components";
import { useNav, useStrap } from "../lib/app-context";
import { addDays, fmtTime, today } from "../lib/date";

const EXPORTS: { value: ExportKind; label: string }[] = [
  { value: "heart_rate", label: "Heart rate" }, { value: "rr_intervals", label: "R-R intervals" }, { value: "daily_summary", label: "Daily summary" },
  { value: "sleeps", label: "Sleeps" }, { value: "activities", label: "Activities" }, { value: "stress", label: "Stress" }, { value: "raw_frames", label: "Raw frames" },
];

export function Strap() {
  const { status, progress, liveHr, rawLog, clearRawLog } = useStrap();
  const nav = useNav();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [kind, setKind] = useState<ExportKind>("heart_rate");
  const [from, setFrom] = useState(addDays(today(), -30));
  const [to, setTo] = useState(today());
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => { api.getSettings().then(setSettings).catch((e) => setErr(String(e))); }, []);
  useEffect(() => { const el = logRef.current; if (el) el.scrollTop = el.scrollHeight; }, [rawLog]);

  const run = async (fn: () => Promise<unknown>, ok?: string) => {
    setBusy(true); setErr(null); setMsg(null);
    try { await fn(); if (ok) setMsg(ok); } catch (e) { setErr(String((e as Error)?.message ?? e)); } finally { setBusy(false); }
  };

  if (!settings || !status) return <div className="page"><div className="empty faint">Loading…</div></div>;
  const transport = settings.transport;
  const connected = status.state === "connected" || status.state === "syncing";
  const working = status.state === "scanning" || status.state === "connecting";
  const patch = (p: Partial<Settings>) => setSettings({ ...settings, ...p });
  const save = () => run(async () => { setSettings(await api.setSettings(settings)); }, "Settings saved");
  const syncing = status.state === "syncing" || (progress !== null && progress.phase !== "done");

  return (
    <div className="page">
      <h1 className="page-title">Strap</h1>

      <div className="grid g2">
        <Card title="Connection">
          <div className="col">
            <Segmented<Transport> ariaLabel="Transport" value={transport} onChange={(t) => { patch({ transport: t }); }}
              options={[{ value: "ble", label: "Bluetooth" }, { value: "emulator", label: "Emulator" }]} />
            {transport === "emulator" && (
              <div className="field"><label>Emulator address</label>
                <input value={settings.emulatorAddress} onChange={(e) => patch({ emulatorAddress: e.target.value })} spellCheck={false} />
              </div>
            )}
            <div className="row wrap">
              {connected || working ? (
                <button className="btn" disabled={busy} onClick={() => run(() => api.disconnectStrap())}>Disconnect</button>
              ) : (
                <button className="btn primary" disabled={busy} onClick={() => run(async () => { setSettings(await api.setSettings(settings)); await api.connectStrap(transport); })}>
                  Connect
                </button>
              )}
              <span className="muted">
                {status.state === "connected" ? `Connected to ${status.deviceName ?? "strap"}` : status.state === "syncing" ? "Syncing…" : working ? (status.state === "scanning" ? "Scanning…" : "Connecting…") : status.state === "error" ? "Error" : "Not connected"}
              </span>
            </div>
            {(status.error || err) && <p className="error-text">{status.error ?? err}</p>}
            {msg && <p className="muted">{msg}</p>}
            <div className="grid g3">
              <StatTile label="Battery" value={status.batteryPct === null ? "—" : `${status.batteryPct}%`} sub={status.charging ? "Charging" : undefined} />
              <StatTile label="On wrist" value={status.onWrist === null ? "—" : status.onWrist ? "Yes" : "No"} />
              <StatTile label="Last sync" value={status.lastSyncMs ? fmtTime(status.lastSyncMs) : "—"} />
            </div>
          </div>
        </Card>

        <Card title="Live heart rate">
          {connected && liveHr && status.onWrist !== false ? (
            <div className="col" style={{ gap: 2 }}>
              <div className="live-hr num"><span className="heartbeat">♥</span> {liveHr.bpm}<span className="stat-unit">bpm</span></div>
              <span className="faint num">updated {fmtTime(liveHr.ts)}</span>
            </div>
          ) : <EmptyState title="Live when connected" detail={connected && status.onWrist === false ? "Strap is off the wrist." : "Connect a strap to see live heart rate."} />}
        </Card>
      </div>

      <Card title="Sync">
        <div className="row wrap">
          <button className="btn primary" disabled={!connected || syncing || busy} onClick={() => run(() => api.syncNow())}>Sync now</button>
          {progress && progress.phase !== "idle" ? (
            <>
              <span className="muted">{progress.label}</span>
              <span className="sync-bar" style={{ width: 220 }}><i style={{ width: `${Math.round(progress.fraction * 100)}%` }} /></span>
            </>
          ) : <span className="muted">{status.lastSyncMs ? `Last synced ${new Date(status.lastSyncMs).toLocaleString()}` : "Not synced yet"}</span>}
          <span className="spacer" />
          <button className="btn" disabled={busy || syncing} onClick={() => run(() => api.rebuildDecoded(), "Decoded data rebuilt")}>Rebuild decoded data</button>
        </div>
      </Card>

      <Card title="Raw frames" action={{ label: "Clear", onClick: clearRawLog }}>
        <div className="hex-log" ref={logRef} aria-label="Raw frame log" role="log">
          {rawLog.length === 0 && <span className="faint">Waiting for notifications…</span>}
          {rawLog.map((l, i) => (
            <div key={i}><span className="t">{new Date(l.receivedAt).toLocaleTimeString()}</span><span className="c">{l.characteristic.slice(4, 8)}</span>{l.hex}</div>
          ))}
        </div>
      </Card>

      {transport === "emulator" && (
        <Card title="Emulator controls (dev)">
          <div className="row wrap">
            <button className="btn sm" onClick={() => run(() => api.emulatorControl("out_of_range"))}>Out of range</button>
            <button className="btn sm" onClick={() => run(() => api.emulatorControl("in_range"))}>In range</button>
            <button className="btn sm" onClick={() => run(() => api.emulatorControl("take_off"))}>Take off</button>
            <button className="btn sm" onClick={() => run(() => api.emulatorControl("put_on"))}>Put on</button>
            <button className="btn sm" onClick={() => run(() => api.emulatorControl("advance_minutes", 60))}>Advance 60 min</button>
          </div>
        </Card>
      )}

      <div className="grid g2">
        <Card title="Profile and options">
          <div className="grid g2">
            <div className="field"><label>Age</label><input type="number" min={10} max={110} value={settings.profile.age} onChange={(e) => patch({ profile: { ...settings.profile, age: Number(e.target.value) } })} /></div>
            <div className="field"><label>Sex</label>
              <select value={settings.profile.sex} onChange={(e) => patch({ profile: { ...settings.profile, sex: e.target.value as Settings["profile"]["sex"] } })}>
                <option value="male">Male</option><option value="female">Female</option><option value="other">Other</option>
              </select></div>
            <div className="field"><label>Weight (kg)</label><input type="number" min={30} max={250} value={settings.profile.weightKg} onChange={(e) => patch({ profile: { ...settings.profile, weightKg: Number(e.target.value) } })} /></div>
            <div className="field"><label>Height (cm)</label><input type="number" min={100} max={230} value={settings.profile.heightCm} onChange={(e) => patch({ profile: { ...settings.profile, heightCm: Number(e.target.value) } })} /></div>
          </div>
          <div className="col" style={{ marginTop: 12 }}>
            <label className="row"><input type="checkbox" checked={settings.autoSync} onChange={(e) => patch({ autoSync: e.target.checked })} /> Sync automatically</label>
            <label className="row"><input type="checkbox" checked={settings.launchAtLogin} onChange={(e) => patch({ launchAtLogin: e.target.checked })} /> Launch at login</label>
            <div><button className="btn primary sm" disabled={busy} onClick={save}>Save settings</button></div>
          </div>
        </Card>

        <Card title="Export CSV">
          <div className="col">
            <div className="field"><label>Data</label>
              <select value={kind} onChange={(e) => setKind(e.target.value as ExportKind)}>{EXPORTS.map((x) => <option key={x.value} value={x.value}>{x.label}</option>)}</select></div>
            <div className="grid g2">
              <div className="field"><label>From</label><input type="date" value={from} onChange={(e) => setFrom(e.target.value)} /></div>
              <div className="field"><label>To</label><input type="date" value={to} onChange={(e) => setTo(e.target.value)} /></div>
            </div>
            <div><button className="btn sm" disabled={busy || !from || !to} onClick={() => run(async () => { const p = await api.exportCsv(kind, from, to); setMsg(`Saved to ${p}`); })}>Export</button></div>
            <p className="faint">Also available under <button className="card-action" onClick={() => nav.go("data")}>Data</button>.</p>
          </div>
        </Card>
      </div>
    </div>
  );
}
