mod common;
use common::*;
use unstrapped_core::protocol::DeviceFamily;
use unstrapped_emulator::physio::{Activity, Physiology, Ph};
use unstrapped_emulator::SubjectProfile;

fn phys() -> Physiology {
    let p = SubjectProfile::default();
    Physiology::new(7, p, 0, 20_450)
}

fn day_of(t_ms: i64) -> i64 {
    t_ms.div_euclid(86_400_000)
}

#[test]
fn deterministic() {
    let a = phys();
    let b = phys();
    let t = T0 / 1000 + 12345;
    for k in 0..200 {
        let (x, y) = (a.sample(t + k).unwrap(), b.sample(t + k).unwrap());
        assert_eq!(x.bpm, y.bpm);
        assert_eq!(x.rr_ms, y.rr_ms);
        assert_eq!(x.gravity, y.gravity);
    }
    // order independence: regenerate an earlier second after later ones.
    let first = a.sample(t).unwrap();
    let _ = a.sample(t + 99_999);
    let again = a.sample(t).unwrap();
    assert_eq!(first.rr_ms, again.rr_ms);
    assert_eq!(first.bpm, again.bpm);
}

#[test]
fn night_has_expected_shape() {
    let ph = phys();
    let d = day_of(T0) + 3;
    let plan = ph.day_plan(d);
    // bed ~ 23:00 previous evening, wake ~ 07:00.
    let bed_h = ((plan.bed - ph.day_start(d)) / 3600.0) + 24.0;
    let wake_h = (plan.wake - ph.day_start(d)) / 3600.0;
    assert!((bed_h - 23.0).abs() < 1.2, "bed {bed_h}");
    assert!((wake_h - 7.0).abs() < 1.0, "wake {wake_h}");
    assert!(plan.onset > plan.bed);
    let mut deep_first = 0.0;
    let mut deep_second = 0.0;
    let mut rem_first = 0.0;
    let mut rem_second = 0.0;
    let mid = (plan.onset + plan.wake) / 2.0;
    for s in &plan.segs {
        let l = s.b - s.a;
        let first = s.a < mid;
        match s.ph {
            Ph::Deep => if first { deep_first += l } else { deep_second += l },
            Ph::Rem => if first { rem_first += l } else { rem_second += l },
            _ => {}
        }
    }
    assert!(deep_first > deep_second * 1.5, "deep early: {deep_first} vs {deep_second}");
    assert!(rem_second > rem_first, "rem late: {rem_second} vs {rem_first}");
    let n_wake = plan.segs.iter().filter(|s| s.ph == Ph::Wake).count();
    assert!((2..=4).contains(&n_wake));
}

#[test]
fn sleep_hr_below_day_hr_and_ranges() {
    let ph = phys();
    let d = day_of(T0) + 3;
    let plan = ph.day_plan(d);
    let mut sleep_hr = vec![];
    let mut day_hr = vec![];
    for t in (plan.onset as i64 + 1800..plan.wake as i64 - 600).step_by(5) {
        sleep_hr.push(ph.sample(t).unwrap().bpm as f64);
    }
    let day0 = ph.day_start(d) as i64;
    for t in (plan.wake as i64 + 7200..day0 + 20 * 3600).step_by(5) {
        if let Some(s) = ph.sample(t) {
            day_hr.push(s.bpm as f64);
        }
    }
    let ms = sleep_hr.iter().sum::<f64>() / sleep_hr.len() as f64;
    let md = day_hr.iter().sum::<f64>() / day_hr.len() as f64;
    assert!(ms + 12.0 < md, "sleep {ms} day {md}");
    assert!(sleep_hr.iter().all(|h| (38.0..=100.0).contains(h)));
    assert!(day_hr.iter().all(|h| (40.0..=200.0).contains(h)));
}

#[test]
fn rmssd_and_resp_match_profile() {
    let ph = phys();
    let mut ratios = vec![];
    for d in day_of(T0)..day_of(T0) + 10 {
        let plan = ph.day_plan(d);
        let (a, b) = (plan.onset as i64 + 3600, plan.wake as i64 - 1800);
        let mut rr = vec![];
        for t in a..b {
            if let Some(s) = ph.sample(t) {
                rr.extend(s.rr_ms.iter().map(|x| *x as f64));
            }
        }
        let r = rmssd(&rr);
        ratios.push(r / plan.hrv_night);
        assert!((0.8..1.25).contains(&(r / plan.hrv_night)), "day {d}: rmssd {r} planned {}", plan.hrv_night);
        // RR consistent with HR.
        let mean_rr = rr.iter().sum::<f64>() / rr.len() as f64;
        assert!((40.0..90.0).contains(&(60_000.0 / mean_rr)));
        assert!((12.5..=17.5).contains(&plan.resp_night));
    }
    // Nightly RMSSD is near the profile on average, with real night-to-night variation.
    let mean_hrv: f64 = (day_of(T0)..day_of(T0) + 10).map(|d| ph.day_plan(d).hrv_night).sum::<f64>() / 10.0;
    assert!((45.0..80.0).contains(&mean_hrv), "{mean_hrv}");
}

#[test]
fn variation_rough_high_strain_and_workouts() {
    let ph = phys();
    let days: Vec<_> = (day_of(T0)..day_of(T0) + 14).map(|d| ph.day_plan(d)).collect();
    assert!(days.iter().any(|p| p.rough));
    assert!(days.iter().any(|p| p.high_strain));
    assert!(days.iter().filter(|p| p.workout.is_some()).count() >= 3);
    assert!(days.iter().any(|p| p.off_wrist.is_some()));
    let rough = days.iter().find(|p| p.rough).unwrap();
    let normal_hrv = days.iter().filter(|p| !p.rough).map(|p| p.hrv_night).sum::<f64>() / 13.0;
    assert!(rough.hrv_night < normal_hrv * 0.85);
    assert!(rough.resp_night > 14.4);
}

#[test]
fn motion_levels_follow_activity() {
    for fam in [DeviceFamily::Whoop4, DeviceFamily::Whoop5] {
        let ph = phys();
        let mut by = std::collections::HashMap::<&str, Vec<f64>>::new();
        for d in day_of(T0) + 2..day_of(T0) + 5 {
            let plan = ph.day_plan(d);
            let a = plan.bed as i64 - 3600;
            let b = plan.wake as i64 + 14 * 3600;
            let s = gen_range(&ph, fam, a, b);
            let st = to_streams(&s, fam);
            for m in &st.motion {
                let t = m.ts_ms as f64 / 1000.0;
                let k = match ph.activity_at(t) {
                    Activity::Sleep(Ph::Light | Ph::Deep | Ph::Rem) => "sleep",
                    Activity::Sleep(Ph::Wake) => "wake",
                    Activity::Sleep(Ph::InBed) => "inbed",
                    Activity::Sit => "sit",
                    Activity::Walk => "walk",
                    Activity::Run => "run",
                };
                by.entry(k).or_default().push(m.magnitude_g);
            }
        }
        let mean = |k: &str| {
            let v = &by[k];
            v.iter().sum::<f64>() / v.len() as f64
        };
        println!("{fam:?} sleep {:.4} sit {:.4} walk {:.4} run {:.4}", mean("sleep"), mean("sit"), mean("walk"), mean("run"));
        assert!(mean("sleep") < 0.010, "sleep {}", mean("sleep"));
        assert!((0.010..0.05).contains(&mean("sit")), "sit {}", mean("sit"));
        assert!(mean("walk") > 0.1, "walk {}", mean("walk"));
        assert!(mean("run") > 0.3, "run {}", mean("run"));
    }
}

#[test]
fn steps_and_temperature() {
    let ph = phys();
    let d = day_of(T0) + 3;
    let day0 = ph.day_start(d) as i64;
    let s = gen_range(&ph, DeviceFamily::Whoop5, day0, day0 + 86_400);
    let st = to_streams(&s, DeviceFamily::Whoop5);
    let total: u32 = st.steps.iter().map(|x| x.steps).sum();
    assert!((3_000..30_000).contains(&total), "steps {total}");
    let plan = ph.day_plan(d);
    let night: Vec<f64> = (plan.onset as i64 + 7200..plan.wake as i64 - 1800).step_by(60).map(|t| ph.sample(t).unwrap().temp_c as f64).collect();
    let noon: Vec<f64> = (day0 + 12 * 3600..day0 + 13 * 3600).step_by(60).filter_map(|t| ph.sample(t).map(|s| s.temp_c as f64)).collect();
    let (n, m) = (night.iter().sum::<f64>() / night.len() as f64, noon.iter().sum::<f64>() / noon.len() as f64);
    assert!(n > m + 0.6, "night {n} day {m}");
    assert!((30.0..37.0).contains(&n) && (30.0..37.0).contains(&m));
}

#[test]
fn off_wrist_gap_produces_no_samples() {
    let ph = phys();
    let p = (day_of(T0)..day_of(T0) + 14).map(|d| ph.day_plan(d)).find(|p| p.off_wrist.is_some()).unwrap();
    let (a, b) = p.off_wrist.unwrap();
    assert!(ph.sample(a as i64 - 1).is_some());
    assert!(ph.sample(a as i64 + 5).is_none());
    assert!(ph.sample(b as i64).is_some());
}
