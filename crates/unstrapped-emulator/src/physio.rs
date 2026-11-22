//! Synthetic physiology: a deterministic function of `(seed, profile, timestamp)`.
//!
//! Nothing is stored except small caches: per-day plans (bed/wake times, workouts, sleep structure),
//! a per-day step table and the current minute of beat-to-beat intervals. Every sample can be
//! regenerated for any second, so a strap can "hold" 14 days of 1 Hz data without keeping it.
//!
//! Model summary:
//! * Nights: bedtime/wake jitter, sleep-onset latency, ~90 min cycles (deep early, REM late), 2-4
//!   brief awakenings. Good/bad nights, one "rough" day per 14 (low HRV, high RHR, high resp rate)
//!   and one high-strain day per 14 (long hard run, depressed HRV the next night).
//! * Heart rate follows stage/activity levels; R-R intervals are generated per minute with a
//!   respiratory sinus arrhythmia component (~13-16 breaths/min asleep) so that the series RMSSD
//!   matches the planned HRV.
//! * Motion is encoded as a gravity vector whose second-to-second change has the magnitude the
//!   metrics engine expects (still ~0.004 g asleep, ~0.02 g at a desk, ~0.15 g walking, ~0.4 g running).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use crate::SubjectProfile;
use crate::rng::*;

pub const DAY_S: i64 = 86_400;

/// Sleep / in-bed phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ph {
    InBed,
    Wake,
    Light,
    Deep,
    Rem,
}

/// What the wearer is doing (coarse).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Sleep(Ph),
    Sit,
    Walk,
    Run,
}

#[derive(Clone, Copy, Debug)]
pub struct Seg {
    pub a: f64,
    pub b: f64,
    pub ph: Ph,
}

#[derive(Clone, Debug)]
pub struct Workout {
    pub start: f64,
    pub end: f64,
    pub peak_hr: f64,
    pub high_strain: bool,
}

/// Everything decided about one local day (and the night that ends on it).
#[derive(Clone, Debug)]
pub struct DayPlan {
    /// Local day index (days since 1970-01-01, local calendar).
    pub day: i64,
    /// Night ending on this day: in bed / asleep / awake (Unix seconds).
    pub bed: f64,
    pub onset: f64,
    pub wake: f64,
    pub rough: bool,
    pub high_strain: bool,
    pub hrv_night: f64,
    pub rhr_night: f64,
    pub resp_night: f64,
    pub temp_dev: f64,
    pub segs: Vec<Seg>,
    pub workout: Option<Workout>,
    pub off_wrist: Option<(f64, f64)>,
    day_temp_off: f64,
}

/// One synthetic per-second reading (what a strap would record).
#[derive(Clone, Debug)]
pub struct Sample {
    pub t_s: i64,
    pub bpm: u8,
    /// Beats that ended in this second (ms).
    pub rr_ms: Vec<u16>,
    pub temp_c: f32,
    /// Unit-ish gravity vector in g.
    pub gravity: [f32; 3],
    /// Motion magnitude the generator aimed for (g per second).
    pub motion_g: f32,
    /// Cumulative step counter (wraps at 65536).
    pub steps: u32,
    pub spo2_pct: f32,
    pub spo2_red: u16,
    pub spo2_ir: u16,
    pub activity: Activity,
}

#[derive(Clone, Copy)]
struct Level {
    hr_add: f64,
    motion: f64,
    cadence: f64,
    hrv: f64,
}

fn mix(a: Level, b: Level, t: f64) -> Level {
    Level {
        hr_add: lerp(a.hr_add, b.hr_add, t),
        motion: lerp(a.motion, b.motion, t),
        cadence: lerp(a.cadence, b.cadence, t),
        hrv: lerp(a.hrv, b.hrv, t),
    }
}

struct Core {
    hr: f64,
    hr_noise: f64,
    motion: f64,
    rmssd: f64,
    resp_f: f64,
    resp_irreg: f64,
    act: Activity,
    spo2: f64,
}

#[derive(Clone, Copy)]
struct Beat {
    end_s: f64,
    rr: f64,
}

struct StepsDay {
    base: u64,
    prefix: Vec<u32>, // 1441 entries
}

pub struct Physiology {
    seed: u64,
    profile: SubjectProfile,
    tz_s: i64,
    anchor_day: i64,
    plans: RefCell<HashMap<i64, Arc<DayPlan>>>,
    steps: RefCell<HashMap<i64, Arc<StepsDay>>>,
    block: RefCell<Option<(i64, Arc<Vec<Beat>>)>>,
}

type NightPrm = (f64, f64, f64, f64);

fn night_params(ph: Ph) -> NightPrm {
    // (hr_add over night RHR, motion g, hrv multiplier, breathing irregularity 0..1)
    match ph {
        Ph::InBed => (10.0, 0.008, 0.7, 0.5),
        Ph::Wake => (14.0, 0.10, 0.5, 0.8),
        Ph::Light => (1.5, 0.004, 1.0, 0.3),
        Ph::Deep => (-3.0, 0.002, 1.4, 0.0),
        Ph::Rem => (4.0, 0.003, 0.8, 1.0),
    }
}

fn is_rough(seed: u64, d: i64) -> bool {
    d.rem_euclid(14) == 11 || u01(seed, 1, d) < 0.05
}

fn is_high(seed: u64, d: i64) -> bool {
    d.rem_euclid(14) == 6 || u01(seed, 2, d) < 0.03
}

impl Physiology {
    /// `anchor_day`: local day the step counter starts from zero (strap "boot" day).
    pub fn new(seed: u64, profile: SubjectProfile, tz_offset_min: i32, anchor_day: i64) -> Self {
        Physiology {
            seed,
            profile,
            tz_s: tz_offset_min as i64 * 60,
            anchor_day,
            plans: RefCell::new(HashMap::new()),
            steps: RefCell::new(HashMap::new()),
            block: RefCell::new(None),
        }
    }

    pub fn profile(&self) -> &SubjectProfile {
        &self.profile
    }

    pub fn tz_offset_min(&self) -> i32 {
        (self.tz_s / 60) as i32
    }

    /// Local day index of a Unix-second timestamp.
    pub fn local_day(&self, t: f64) -> i64 {
        ((t.floor() as i64) + self.tz_s).div_euclid(DAY_S)
    }

    /// Unix second of local midnight starting local day `d`.
    pub fn day_start(&self, d: i64) -> f64 {
        (d * DAY_S - self.tz_s) as f64
    }

    pub fn day_plan(&self, d: i64) -> Arc<DayPlan> {
        if let Some(p) = self.plans.borrow().get(&d) {
            return p.clone();
        }
        let p = Arc::new(self.build_plan(d));
        let mut m = self.plans.borrow_mut();
        if m.len() > 400 {
            m.clear();
        }
        m.insert(d, p.clone());
        p
    }

    fn build_plan(&self, d: i64) -> DayPlan {
        let seed = self.seed;
        let p = &self.profile;
        let day0 = self.day_start(d);
        let rough = is_rough(seed, d);
        let high = is_high(seed, d);
        let after_high = is_high(seed, d - 1);
        let z = gauss(seed, 3, d);
        let bed_off = if p.bedtime_min >= p.wake_min { -1440.0 } else { 0.0 };
        let mut bed = day0 + (p.bedtime_min as f64 + bed_off) * 60.0
            + (gauss(seed, 4, d) * 22.0).clamp(-50.0, 50.0) * 60.0
            + if rough { 25.0 * 60.0 } else { 0.0 };
        let wake = day0 + p.wake_min as f64 * 60.0
            + (gauss(seed, 5, d) * 15.0).clamp(-35.0, 35.0) * 60.0
            + if rough { 20.0 * 60.0 } else { 0.0 };
        if wake - bed < 5.5 * 3600.0 {
            bed = wake - 5.5 * 3600.0;
        }
        let latency = (6.0 + 6.0 * gauss(seed, 6, d).abs() + if rough { 8.0 } else { 0.0 }).clamp(3.0, 40.0);
        let onset = bed + latency * 60.0;

        let mut hrv = p.hrv_rmssd_ms * (0.2 * z).exp();
        let mut rhr = p.resting_hr_bpm + 2.0 - 1.5 * z;
        if rough {
            hrv *= 0.68;
            rhr += 2.5;
        }
        if after_high {
            hrv *= 0.9;
            rhr += 2.0;
        }
        let resp = (14.4 + 0.5 * gauss(seed, 7, d) + if rough { 1.6 } else { 0.0 }).clamp(12.5, 17.5);
        let temp_dev = 0.12 * gauss(seed, 8, d) + if rough { 0.3 } else { 0.0 };

        // Sleep structure.
        let mut segs = vec![Seg { a: bed, b: onset, ph: Ph::InBed }];
        let mut t = onset;
        let mut k = 0i64;
        while t < wake {
            let len = ((90.0 + 7.0 * gauss(seed, 9, d * 32 + k)).clamp(75.0, 105.0)) * 60.0;
            let dk = (0.30 - 0.07 * k as f64).max(0.04);
            let rk = (0.10 + 0.06 * k as f64).min(0.40);
            let parts = [(0.12, Ph::Light), (dk, Ph::Deep), (0.18, Ph::Light), (rk, Ph::Rem)];
            let mut c = t;
            for (f, ph) in parts {
                let e = (c + f * len).min(wake);
                if e > c {
                    segs.push(Seg { a: c, b: e, ph });
                }
                c = e;
            }
            let e = (t + len).min(wake);
            if e > c {
                segs.push(Seg { a: c, b: e, ph: Ph::Light });
            }
            t += len;
            k += 1;
        }
        let n_awake = 2 + (hash(seed, 10, d) % 3) as i64;
        for i in 0..n_awake {
            let span = (wake - onset - 70.0 * 60.0).max(600.0);
            let s = onset + 40.0 * 60.0 + u01(seed, 11, d * 8 + i) * span;
            let dur = 60.0 + u01(seed, 12, d * 8 + i) * 180.0;
            overlay(&mut segs, s, (s + dur).min(wake), Ph::Wake);
        }

        // Workout.
        let est_bed = day0 + (if p.bedtime_min >= p.wake_min { p.bedtime_min as f64 } else { p.bedtime_min as f64 + 1440.0 }) * 60.0;
        let has_workout = high || u01(seed, 13, d) < 0.42;
        let workout = has_workout.then(|| {
            let dur = if high { (75.0 + 25.0 * u01(seed, 14, d)) * 60.0 } else { (30.0 + 25.0 * u01(seed, 14, d)) * 60.0 };
            let lo = wake + 75.0 * 60.0;
            let hi = (est_bed - 90.0 * 60.0 - dur).max(lo);
            let center = if u01(seed, 15, d) < 0.65 { day0 + 17.75 * 3600.0 + (u01(seed, 16, d) - 0.5) * 2.5 * 3600.0 } else { wake + 90.0 * 60.0 + u01(seed, 16, d) * 40.0 * 60.0 };
            let start = center.clamp(lo, hi).floor();
            let frac = if high { 0.86 } else { 0.72 + 0.12 * u01(seed, 17, d) };
            Workout { start, end: start + dur.floor(), peak_hr: (p.max_hr_bpm * frac).min(p.max_hr_bpm - 2.0), high_strain: high }
        });

        // Strap-off gap (shower etc.), 35% of days, 10:00-15:00 local, away from workouts/sleep.
        let off_wrist = (u01(seed, 18, d) < 0.35).then(|| {
            let s = (day0 + (10.0 + 4.5 * u01(seed, 19, d)) * 3600.0).floor();
            (s, s + ((20.0 + 30.0 * u01(seed, 20, d)) * 60.0).floor())
        });
        let off_wrist = match (&workout, off_wrist) {
            (Some(w), Some((a, b))) if a < w.end + 600.0 && b > w.start - 600.0 => None,
            (_, o) => o,
        };

        DayPlan {
            day: d,
            bed,
            onset,
            wake,
            rough,
            high_strain: high,
            hrv_night: hrv.clamp(15.0, 200.0),
            rhr_night: rhr.clamp(35.0, 90.0),
            resp_night: resp,
            temp_dev,
            segs,
            workout,
            off_wrist,
            day_temp_off: 0.15 * gauss(seed, 21, d),
        }
    }

    /// Is the strap on the wrist (ignoring manual controls)?
    pub fn on_wrist(&self, t: i64) -> bool {
        let p = self.day_plan(self.local_day(t as f64));
        match p.off_wrist {
            Some((a, b)) => !((t as f64) >= a && (t as f64) < b),
            None => true,
        }
    }

    /// The night containing `t` (in bed through wake), with the index of its segment.
    fn night_at(&self, t: f64) -> Option<(Arc<DayPlan>, usize)> {
        let d0 = self.local_day(t);
        for d in [d0, d0 + 1] {
            let p = self.day_plan(d);
            if t >= p.bed && t < p.wake {
                let i = p.segs.iter().position(|s| t >= s.a && t < s.b)?;
                return Some((p, i));
            }
        }
        None
    }

    fn local_hour(&self, t: f64) -> f64 {
        ((t + self.tz_s as f64).rem_euclid(DAY_S as f64)) / 3600.0
    }

    fn hr_sit(&self, t: f64, rough: bool) -> f64 {
        let lh = self.local_hour(t);
        self.profile.resting_hr_bpm + 26.0 + 3.0 * (std::f64::consts::TAU * (lh - 9.0) / 24.0).sin() + if rough { 3.0 } else { 0.0 }
    }

    fn bucket_level(&self, bucket: i64) -> Level {
        let u = u01(self.seed, 30, bucket);
        let v = u01(self.seed, 31, bucket);
        if u < 0.09 {
            Level { hr_add: 20.0 + 12.0 * v, motion: 0.14 + 0.04 * v, cadence: 98.0 + 14.0 * v, hrv: 0.35 }
        } else if u < 0.11 {
            // brisk walk / stairs
            Level { hr_add: 42.0 + 10.0 * v, motion: 0.22, cadence: 122.0, hrv: 0.3 }
        } else {
            Level { hr_add: 0.0, motion: 0.015 + 0.012 * v, cadence: 0.0, hrv: 0.55 }
        }
    }

    fn day_level(&self, t: f64) -> Level {
        let tl = t + self.tz_s as f64;
        let b = (tl / 600.0).floor() as i64;
        let pos = tl - b as f64 * 600.0;
        let cur = self.bucket_level(b);
        if pos < 60.0 {
            mix(self.bucket_level(b - 1), cur, 0.5 + 0.5 * pos / 60.0)
        } else if pos > 540.0 {
            mix(cur, self.bucket_level(b + 1), 0.5 * (pos - 540.0) / 60.0)
        } else {
            cur
        }
    }

    fn slow_var(&self, t: f64, amp: f64) -> f64 {
        let ph = |tag: u64| u01(self.seed, 40 + tag, 0) * std::f64::consts::TAU;
        amp * (0.6 * (std::f64::consts::TAU * t / 263.0 + ph(0)).sin() + 0.4 * (std::f64::consts::TAU * t / 677.0 + ph(1)).sin())
    }

    fn core(&self, t: f64) -> Core {
        let resp_phase = |plan_resp: f64| plan_resp / 60.0;
        if let Some((plan, i)) = self.night_at(t) {
            let seg = plan.segs[i];
            let cur = night_params(seg.ph);
            let mut prm = cur;
            let blend = |a: NightPrm, b: NightPrm, w: f64| (lerp(a.0, b.0, w), lerp(a.1, b.1, w), lerp(a.2, b.2, w), lerp(a.3, b.3, w));
            if i > 0 && t - seg.a < 90.0 {
                prm = blend(night_params(plan.segs[i - 1].ph), cur, 0.5 + 0.5 * (t - seg.a) / 90.0);
            } else if i + 1 < plan.segs.len() && seg.b - t < 90.0 {
                prm = blend(cur, night_params(plan.segs[i + 1].ph), 0.5 * (1.0 - (seg.b - t) / 90.0));
            }
            let x = ((t - plan.onset) / (plan.wake - plan.onset)).clamp(0.0, 1.0);
            let trend = 2.5 * (1.0 - (std::f64::consts::PI * x).sin());
            let bump = if t >= plan.onset { 9.0 * (-(t - plan.onset) / 1500.0).exp() } else { 0.0 };
            let rem_var = if seg.ph == Ph::Rem { self.slow_var(t * 3.1, 5.0) } else { 0.0 };
            let hr = plan.rhr_night + prm.0 + trend + bump + self.slow_var(t, 1.2) + rem_var;
            let act = if seg.ph == Ph::InBed { Activity::Sleep(Ph::InBed) } else { Activity::Sleep(seg.ph) };
            return Core {
                hr,
                hr_noise: 0.8,
                motion: prm.1,
                rmssd: plan.hrv_night * prm.2,
                resp_f: resp_phase(plan.resp_night),
                resp_irreg: prm.3,
                act,
                spo2: 96.6 + 0.6 * self.slow_var(t, 1.0) - if seg.ph == Ph::Wake { 0.4 } else { 0.0 },
            };
        }
        // Daytime.
        let d = self.local_day(t);
        let plan = self.day_plan(d);
        let hr_sit = self.hr_sit(t, plan.rough);
        let mut lvl = self.day_level(t);
        let mut act = if lvl.cadence > 40.0 { Activity::Walk } else { Activity::Sit };
        let mut extra = 0.0;
        if let Some(w) = &plan.workout {
            let w_in = if t >= w.start && t <= w.end { ((t - w.start) / 240.0).min((w.end - t) / 180.0).clamp(0.0, 1.0) } else { 0.0 };
            if w_in > 0.0 {
                let ramp = smoothstep((t - w.start) / 900.0);
                let mut target = w.peak_hr * (0.86 + 0.14 * ramp);
                if w.high_strain {
                    target += 5.0 * (std::f64::consts::TAU * (t - w.start) / 480.0).sin();
                }
                let run = Level { hr_add: target - hr_sit, motion: 0.42, cadence: 165.0, hrv: 0.12 };
                lvl = mix(lvl, run, w_in);
                if w_in > 0.5 {
                    act = Activity::Run;
                }
            } else if t > w.end {
                extra = (w.peak_hr - hr_sit).max(0.0) * 0.3 * (-(t - w.end) / 900.0).exp();
            }
        }
        let hr = hr_sit + lvl.hr_add + extra + self.slow_var(t, 2.5);
        Core {
            hr,
            hr_noise: if act == Activity::Run { 2.5 } else { 1.5 },
            motion: lvl.motion,
            rmssd: plan.hrv_night * lvl.hrv,
            resp_f: resp_phase(15.5),
            resp_irreg: 0.3,
            act,
            spo2: 97.6 + 0.4 * self.slow_var(t, 1.0),
        }
    }

    // ---- beats ----

    fn block(&self, m: i64) -> Arc<Vec<Beat>> {
        if let Some((bm, b)) = &*self.block.borrow()
            && *bm == m {
                return b.clone();
            }
        let b = Arc::new(self.gen_block(m));
        *self.block.borrow_mut() = Some((m, b.clone()));
        b
    }

    fn gen_block(&self, m: i64) -> Vec<Beat> {
        let start = (m * 60) as f64;
        let mut t = start;
        let mut raw: Vec<f64> = Vec::with_capacity(100);
        let mut sum = 0.0;
        let tau = std::f64::consts::TAU;
        let ph0 = u01(self.seed, 50, 0) * tau;
        let ph1 = u01(self.seed, 51, 0) * tau;
        let ph2 = u01(self.seed, 53, 0) * tau;
        let mut i = 0i64;
        while t < start + 60.0 {
            let c = self.core(t);
            let base = 60_000.0 / c.hr.clamp(30.0, 220.0);
            let f = c.resp_f;
            let r = c.rmssd.max(1.0);
            let k_rsa = (2f64.sqrt() * (std::f64::consts::PI * f * base / 1000.0).sin().abs()).max(0.25);
            let f_lf = 0.09;
            let k_lf = (2f64.sqrt() * (std::f64::consts::PI * f_lf * base / 1000.0).sin().abs()).max(0.15);
            let a = (0.55f64).sqrt() * r / k_rsa;
            let b = (0.10f64).sqrt() * r / k_lf;
            let sn = (0.35f64).sqrt() * r / 2f64.sqrt();
            let rr = base + a * (tau * f * t + ph0 + c.resp_irreg * 1.4 * (tau * t / 41.0 + ph2).sin()).sin() + b * (tau * f_lf * t + ph1).sin() + sn * gauss(self.seed, 52, m * 1000 + i);
            let rr = rr.clamp(300.0, 2000.0);
            raw.push(rr);
            sum += rr;
            t += rr / 1000.0;
            i += 1;
        }
        // Drop the last beat if it overshoots by more than half an interval, then scale so the
        // block spans (just under) exactly 60 s: blocks join seamlessly without an irregular beat.
        if raw.len() > 1 && t - (start + 60.0) > raw[raw.len() - 1] / 2000.0 {
            sum -= raw.pop().unwrap();
        }
        let scale = 59_999.0 / sum;
        let mut e = start;
        raw.into_iter()
            .map(|rr| {
                let rr = rr * scale;
                e += rr / 1000.0;
                Beat { end_s: e, rr }
            })
            .collect()
    }

    /// R-R intervals (ms) of beats that ended during second `t`.
    pub fn beats_in_second(&self, t: i64) -> Vec<u16> {
        let b = self.block(t.div_euclid(60));
        b.iter().filter(|x| x.end_s.floor() as i64 == t).map(|x| x.rr.round() as u16).collect()
    }

    // ---- other channels ----

    fn night_weight(&self, t: f64) -> f64 {
        let d0 = self.local_day(t);
        let mut w: f64 = 0.0;
        for d in [d0, d0 + 1] {
            let p = self.day_plan(d);
            let up = smoothstep((t - (p.onset - 1800.0)) / 3600.0);
            let down = 1.0 - smoothstep((t - p.wake) / 2400.0);
            w = w.max(up * down);
        }
        w
    }

    fn temp_c(&self, t: f64) -> f64 {
        let lh = self.local_hour(t);
        let d = self.local_day(t);
        let plan = self.day_plan(d);
        let w = self.night_weight(t);
        let min = (t / 60.0).floor();
        let n0 = gauss(self.seed, 60, min as i64);
        let n1 = gauss(self.seed, 60, min as i64 + 1);
        let noise = 0.04 * lerp(n0, n1, t / 60.0 - min);
        let dev = if lh < 14.0 { plan.temp_dev } else { self.day_plan(d + 1).temp_dev };
        33.2 + 0.3 * (std::f64::consts::TAU * (lh - 4.0) / 24.0).cos() + 1.1 * w + dev * w + plan.day_temp_off + 0.08 * self.slow_var(t * 0.7, 1.0) + noise
    }

    fn orientation(&self, t: f64, act: Activity, motion: f64) -> [f64; 3] {
        let (len, axis, spread): (f64, [f64; 3], f64) = match act {
            Activity::Sleep(_) => (1500.0, [0.15, 0.2, 0.97], 0.30),
            Activity::Run => (4.0, [0.3, -0.4, 0.85], 1.0),
            _ => (600.0, [0.3, -0.4, 0.85], 0.6),
        };
        let b = (t / len).floor() as i64;
        let u = unit_vec(self.seed, 70, b);
        let o = normalize([axis[0] + spread * u[0], axis[1] + spread * u[1], axis[2] + spread * u[2]]);
        let sigma = motion / 1.85;
        let k = t.floor() as i64;
        normalize([
            o[0] + sigma * gauss(self.seed, 71, k * 3),
            o[1] + sigma * gauss(self.seed, 71, k * 3 + 1),
            o[2] + sigma * gauss(self.seed, 71, k * 3 + 2),
        ])
    }

    // ---- steps ----

    fn minute_steps(&self, d: i64, m: i64) -> u32 {
        let tm = self.day_start(d) + m as f64 * 60.0 + 30.0;
        for dd in [d, d + 1] {
            let p = self.day_plan(dd);
            if tm >= p.bed && tm < p.wake {
                return 0;
            }
        }
        let p = self.day_plan(d);
        if let Some(w) = &p.workout
            && tm >= w.start + 120.0 && tm <= w.end - 90.0 {
                return (165.0 + 6.0 * gauss(self.seed, 80, d * 2000 + m)).round().max(0.0) as u32;
            }
        let tl = tm + self.tz_s as f64;
        let l = self.bucket_level((tl / 600.0).floor() as i64);
        l.cadence.round() as u32
    }

    fn steps_day(&self, d: i64) -> Arc<StepsDay> {
        if let Some(s) = self.steps.borrow().get(&d) {
            return s.clone();
        }
        // Walk forward from the nearest cached earlier day (or the anchor).
        let mut start = d;
        {
            let map = self.steps.borrow();
            while start > self.anchor_day && !map.contains_key(&(start - 1)) {
                start -= 1;
            }
        }
        let mut cur = start;
        loop {
            let base = if cur <= self.anchor_day {
                0
            } else {
                let prev = self.steps.borrow().get(&(cur - 1)).cloned();
                match prev {
                    Some(p) => p.base + *p.prefix.last().unwrap() as u64,
                    None => 0,
                }
            };
            let mut prefix = Vec::with_capacity(1441);
            let mut acc = 0u32;
            prefix.push(0);
            for m in 0..1440 {
                acc += self.minute_steps(cur, m);
                prefix.push(acc);
            }
            let sd = Arc::new(StepsDay { base, prefix });
            self.steps.borrow_mut().insert(cur, sd.clone());
            if cur >= d {
                return sd;
            }
            cur += 1;
        }
    }

    /// Cumulative step counter (u16 wrap) at `t`.
    pub fn steps_counter(&self, t: f64) -> u32 {
        let d = self.local_day(t);
        let sd = self.steps_day(d);
        let sec = t - self.day_start(d);
        let m = ((sec / 60.0).floor() as usize).min(1439);
        let frac = (sec / 60.0 - m as f64).clamp(0.0, 1.0);
        let within = sd.prefix[m] as f64 + frac * (sd.prefix[m + 1] - sd.prefix[m]) as f64;
        ((sd.base + within.floor() as u64) & 0xFFFF) as u32
    }

    /// The activity at `t` (for tests and tooling).
    pub fn activity_at(&self, t: f64) -> Activity {
        self.core(t).act
    }

    /// One per-second sample, or `None` while the strap is off the wrist.
    pub fn sample(&self, t_s: i64) -> Option<Sample> {
        if !self.on_wrist(t_s) {
            return None;
        }
        let t = t_s as f64 + 0.5;
        let c = self.core(t);
        let bpm = (c.hr + c.hr_noise * gauss(self.seed, 90, t_s)).round().clamp(30.0, 215.0) as u8;
        let g = self.orientation(t, c.act, c.motion);
        let pct = (c.spo2 + 0.3 * gauss(self.seed, 91, t_s / 30)).clamp(88.0, 100.0);
        let ir = 24_000.0 + 400.0 * gauss(self.seed, 92, t_s);
        let ratio = (110.0 - pct) / 25.0;
        Some(Sample {
            t_s,
            bpm,
            rr_ms: self.beats_in_second(t_s),
            temp_c: self.temp_c(t) as f32,
            gravity: [g[0] as f32, g[1] as f32, g[2] as f32],
            motion_g: c.motion as f32,
            steps: self.steps_counter(t),
            spo2_pct: pct as f32,
            spo2_red: (ir * ratio).round() as u16,
            spo2_ir: ir.round() as u16,
            activity: c.act,
        })
    }
}

/// Convert the emulator's raw SpO2 red/IR words back to a percentage (emulator convention, not a
/// WHOOP-published calibration): `R = red / ir`, `SpO2 = 110 - 25 R`.
pub fn spo2_from_raw(red: u16, ir: u16) -> f64 {
    if ir == 0 {
        return f64::NAN;
    }
    110.0 - 25.0 * (red as f64 / ir as f64)
}

fn overlay(segs: &mut Vec<Seg>, a: f64, b: f64, ph: Ph) {
    if b <= a {
        return;
    }
    let mut out = Vec::with_capacity(segs.len() + 2);
    for s in segs.iter() {
        if s.ph == Ph::InBed || s.b <= a || s.a >= b {
            out.push(*s);
            continue;
        }
        if s.a < a {
            out.push(Seg { a: s.a, b: a, ph: s.ph });
        }
        out.push(Seg { a: s.a.max(a), b: s.b.min(b), ph });
        if s.b > b {
            out.push(Seg { a: b, b: s.b, ph: s.ph });
        }
    }
    // merge adjacent equal phases
    let mut merged: Vec<Seg> = Vec::with_capacity(out.len());
    for s in out {
        match merged.last_mut() {
            Some(l) if l.ph == s.ph && (l.b - s.a).abs() < 1e-9 => l.b = s.b,
            _ => merged.push(s),
        }
    }
    *segs = merged;
}
