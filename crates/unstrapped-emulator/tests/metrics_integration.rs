//! Feed emulated days (decoded through the real history-record codec) into unstrapped-core's metrics.
mod common;
use common::*;
use unstrapped_core::metrics::{DailySummary, DayInput, RawStreams, ScoreResult, Sex, UserProfile, compute_day, util::format_day};
use unstrapped_core::protocol::DeviceFamily;
use unstrapped_emulator::SubjectProfile;
use unstrapped_emulator::physio::Physiology;

fn run(family: DeviceFamily) {
    let profile = SubjectProfile::default();
    let ph = Physiology::new(7, profile.clone(), 0, 20_000);
    let d0 = 20_460i64; // 2026-01-04 (UTC)
    let days = 6;
    let from = (ph.day_start(d0) as i64) - 6 * 3600;
    let to = ph.day_start(d0 + days) as i64;
    let samples = gen_range(&ph, family, from, to);
    let st = to_streams(&samples, family);
    println!("{family:?}: {} samples, {} rr beats", samples.len(), st.rr.len());
    let user = UserProfile { age: 32.0, sex: Sex::Male, weight_kg: 78.0, height_cm: None, max_hr_override: Some(profile.max_hr_bpm) };

    let mut history: Vec<DailySummary> = vec![];
    let mut results = vec![];
    for i in 0..days {
        let d = d0 + i;
        let day = format_day(d);
        let res = compute_day(&DayInput {
            day: &day,
            tz_offset_min: 0,
            profile: &user,
            streams: RawStreams { hr: &st.hr, rr: &st.rr, motion: &st.motion, temp: &st.temp, spo2: &st.spo2, steps: &st.steps },
            history: &history,
        });
        history.push(res.summary.clone());
        results.push((d, res));
    }

    let mut any_workout = false;
    for (d, res) in &results {
        let plan = ph.day_plan(*d);
        let sl = res.sleeps.iter().find(|s| !s.is_nap).unwrap_or_else(|| panic!("{family:?} day {d}: no main sleep; notes {:?}", res.notes));
        println!(
            "{} rough={} hrv_plan={:.0} | sleep {:.1}h onset_err={:.0}m end_err={:.0}m deep={} rem={} hrv={:?} rhr={:?} resp={:?} | rec={:?} strain={:?} act={} steps={:?}",
            res.day, plan.rough, plan.hrv_night,
            sl.asleep_min as f64 / 60.0,
            (sl.onset_ms as f64 / 1000.0 - plan.onset) / 60.0,
            (sl.end_ms as f64 / 1000.0 - plan.wake) / 60.0,
            sl.deep_min, sl.rem_min, sl.hrv_ms, sl.resting_hr, sl.resp_rate,
            res.summary.recovery, res.summary.day_strain, res.activities.len(), res.summary.steps
        );
        // Sleep detected near the configured bed / wake times.
        assert!((sl.onset_ms as f64 / 1000.0 - plan.onset).abs() < 45.0 * 60.0, "onset");
        assert!((sl.start_ms as f64 / 1000.0 - plan.bed).abs() < 60.0 * 60.0, "in-bed start");
        assert!((sl.end_ms as f64 / 1000.0 - plan.wake).abs() < 30.0 * 60.0, "wake");
        assert!((5 * 60..=9 * 60).contains(&sl.asleep_min), "asleep {}", sl.asleep_min);
        assert!(sl.deep_min > 20 && sl.rem_min > 30, "stages present");
        let hrv = sl.hrv_ms.expect("hrv");
        assert!((hrv / plan.hrv_night - 1.0).abs() < 0.4, "hrv {hrv} vs plan {}", plan.hrv_night);
        let rr = sl.resp_rate.expect("resp");
        assert!((10.0..20.0).contains(&rr), "resp {rr}");
        let rhr = sl.resting_hr.expect("rhr");
        assert!((40.0..75.0).contains(&rhr), "rhr {rhr}");
        let strain = res.summary.day_strain.expect("strain");
        assert!((0.0..=21.0).contains(&strain), "strain {strain}");
        if let Some(w) = &plan.workout {
            let hit = res.activities.iter().any(|a| (a.start_ms as f64 / 1000.0 - w.start).abs() < 15.0 * 60.0 && a.end_ms as f64 / 1000.0 > w.start + 15.0 * 60.0);
            assert!(hit, "{family:?} day {}: planned workout {}..{} not detected; got {:?}", res.day, w.start, w.end, res.activities.iter().map(|a| (a.start_ms / 1000, a.end_ms / 1000)).collect::<Vec<_>>());
            any_workout = true;
            assert!(strain > 6.0, "workout day strain {strain}");
        }
        assert!(res.summary.steps.unwrap_or(0) >= 0);
    }
    assert!(any_workout, "test window must contain a workout day");
    let last = &results.last().unwrap().1;
    match &last.recovery {
        ScoreResult::Score(_) => {}
        other => panic!("recovery should be scored after 5 days of history, got {other:?}"),
    }
    let rec = last.summary.recovery.unwrap();
    assert!((0.0..=100.0).contains(&rec));
    if family == DeviceFamily::Whoop5 {
        assert!(results.iter().any(|(_, r)| r.summary.steps.unwrap_or(0) > 2000), "W5 steps flow into daily summary");
    }
}

#[test]
fn whoop4_days_through_metrics() {
    run(DeviceFamily::Whoop4);
}

#[test]
fn whoop5_days_through_metrics() {
    run(DeviceFamily::Whoop5);
}
