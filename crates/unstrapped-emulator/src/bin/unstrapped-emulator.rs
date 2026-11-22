//! `cargo run -p unstrapped-emulator -- --family whoop4 --time-scale 60`
//!
//! Subcommand: `unstrapped-emulator ctl [--addr A] <action> [value]` sends a dev control
//! (out_of_range, in_range, take_off, put_on, advance_minutes, set_battery, charge_on, charge_off).

use unstrapped_core::metrics::util::days_from_civil;
use unstrapped_emulator::client::EmulatorClient;
use unstrapped_emulator::server::{ServerOptions, serve_with};
use unstrapped_emulator::{DEFAULT_ADDR, DeviceFamily, EmulatorConfig, StrapOptions, SubjectProfile};

const HELP: &str = "\
unstrapped-emulator: a stand-in WHOOP 4.0 / 5.0 strap served over a local TCP socket.

USAGE:
  unstrapped-emulator [OPTIONS]
  unstrapped-emulator ctl [--addr ADDR] <action> [value]

OPTIONS:
  --addr HOST:PORT        listen address (default 127.0.0.1:47800)
  --family whoop4|whoop5  strap generation (default whoop4)
  --seed N                RNG seed (default 42)
  --history-days N        days of history on the strap, 1-14 (default 14)
  --time-scale X          emulated seconds per real second (default 1)
  --start MS|RFC3339      emulated start time (default: now)
  --fast-history          stream history as fast as the host acks (default: BLE-like ~150 rec/s)
  --record-interval S     seconds between stored history records (default 1)
  --tz-offset MIN         local timezone offset in minutes east of UTC (default: system)
  --name NAME             advertised device name
  --age N  --sex male|female  --weight KG  --rhr BPM  --hrv MS  --max-hr BPM
  --bedtime HH:MM  --wake HH:MM
ctl actions: out_of_range in_range take_off put_on advance_minutes <min> set_battery <pct> charge_on charge_off
";

fn die(m: &str) -> ! {
    eprintln!("error: {m}\n\n{HELP}");
    std::process::exit(2);
}

fn hhmm(s: &str) -> u32 {
    let (h, m) = s.split_once(':').unwrap_or_else(|| die("time must be HH:MM"));
    h.parse::<u32>().unwrap_or_else(|_| die("bad hour")) * 60 + m.parse::<u32>().unwrap_or_else(|_| die("bad minute"))
}

/// Unix ms from an integer or an RFC3339 timestamp.
fn parse_start(s: &str) -> i64 {
    if let Ok(v) = s.parse::<i64>() {
        return v;
    }
    let bad = || -> ! { die("--start must be unix ms or RFC3339 (2026-01-01T00:00:00Z)") };
    let (date, rest) = s.split_once(['T', ' ']).unwrap_or_else(|| bad());
    let mut d = date.split('-').map(|x| x.parse::<i64>().unwrap_or_else(|_| bad()));
    let (y, mo, da) = (d.next().unwrap_or_else(|| bad()), d.next().unwrap_or_else(|| bad()), d.next().unwrap_or_else(|| bad()));
    let (time, off_min) = if let Some(t) = rest.strip_suffix('Z') {
        (t, 0)
    } else if let Some(i) = rest.rfind(['+', '-']).filter(|&i| i >= 8) {
        let sign = if rest.as_bytes()[i] == b'-' { -1 } else { 1 };
        let o = &rest[i + 1..];
        let (oh, om) = o.split_once(':').unwrap_or((&o[..2.min(o.len())], o.get(2..).unwrap_or("0")));
        (&rest[..i], sign * (oh.parse::<i64>().unwrap_or(0) * 60 + om.parse::<i64>().unwrap_or(0)))
    } else {
        (rest, 0)
    };
    let mut t = time.split(':');
    let h: i64 = t.next().and_then(|x| x.parse().ok()).unwrap_or_else(|| bad());
    let mi: i64 = t.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let sec: f64 = t.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
    let days = days_from_civil(y, mo as u32, da as u32);
    ((days * 86_400 + h * 3600 + mi * 60 - off_min * 60) as f64 * 1000.0 + sec * 1000.0).round() as i64
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("ctl") {
        return ctl(&args[1..]).await;
    }
    let mut cfg = EmulatorConfig::default();
    let mut addr = DEFAULT_ADDR.to_string();
    let mut sopts = ServerOptions::default();
    let mut strap: StrapOptions = sopts.strap.clone();
    let mut p: SubjectProfile = cfg.profile.clone();
    let mut max_hr_set = false;
    let mut name_set = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let val = |i: &mut usize| -> String {
            *i += 1;
            args.get(*i).cloned().unwrap_or_else(|| die(&format!("{a} needs a value")))
        };
        let f = |s: String| s.parse::<f64>().unwrap_or_else(|_| die(&format!("bad number for {a}")));
        match a {
            "-h" | "--help" => {
                println!("{HELP}");
                return;
            }
            "--addr" => addr = val(&mut i),
            "--family" => {
                cfg.family = match val(&mut i).as_str() {
                    "whoop4" | "4" => DeviceFamily::Whoop4,
                    "whoop5" | "5" => DeviceFamily::Whoop5,
                    _ => die("--family must be whoop4 or whoop5"),
                }
            }
            "--seed" => cfg.seed = val(&mut i).parse().unwrap_or_else(|_| die("bad seed")),
            "--history-days" => cfg.history_days = val(&mut i).parse().unwrap_or_else(|_| die("bad days")),
            "--time-scale" => cfg.time_scale = f(val(&mut i)),
            "--start" => cfg.start_time_ms = Some(parse_start(&val(&mut i))),
            "--fast-history" => sopts.fast_history = true,
            "--record-interval" => strap.record_interval_s = val(&mut i).parse().unwrap_or_else(|_| die("bad interval")),
            "--tz-offset" => strap.tz_offset_min = val(&mut i).parse().unwrap_or_else(|_| die("bad tz offset")),
            "--name" => {
                cfg.device_name = val(&mut i);
                name_set = true;
            }
            "--age" => p.age_years = val(&mut i).parse().unwrap_or_else(|_| die("bad age")),
            "--sex" => p.sex = val(&mut i),
            "--weight" => p.weight_kg = f(val(&mut i)),
            "--rhr" => p.resting_hr_bpm = f(val(&mut i)),
            "--hrv" => p.hrv_rmssd_ms = f(val(&mut i)),
            "--max-hr" => {
                p.max_hr_bpm = f(val(&mut i));
                max_hr_set = true;
            }
            "--bedtime" => p.bedtime_min = hhmm(&val(&mut i)),
            "--wake" => p.wake_min = hhmm(&val(&mut i)),
            other => die(&format!("unknown option {other}")),
        }
        i += 1;
    }
    if !max_hr_set {
        p.max_hr_bpm = (208.0 - 0.7 * p.age_years as f64).round();
    }
    if !name_set && cfg.family == DeviceFamily::Whoop5 {
        cfg.device_name = "WHOOP 5EMU0001".into();
    }
    cfg.profile = p;
    sopts.strap = strap;
    if let Err(e) = serve_with(&addr, cfg, sopts).await {
        eprintln!("emulator failed: {e}");
        std::process::exit(1);
    }
}

async fn ctl(args: &[String]) {
    let mut addr = DEFAULT_ADDR.to_string();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--addr" {
            addr = it.next().cloned().unwrap_or_else(|| die("--addr needs a value"));
        } else {
            rest.push(a.clone());
        }
    }
    let action = rest.first().cloned().unwrap_or_else(|| die("ctl needs an action"));
    let value = rest.get(1).map(|v| v.parse::<f64>().unwrap_or_else(|_| die("value must be a number")));
    let (mut c, _) = EmulatorClient::connect_any(&addr).await.unwrap_or_else(|e| die(&format!("cannot connect to {addr}: {e}")));
    c.control(&action, value).await.unwrap_or_else(|e| die(&e.to_string()));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    println!("sent {action}");
}
