//! Tauri commands: exactly the surface of docs/API_CONTRACT.md. Parameter names are the contract's
//! Rust names (JS camelCase keys are mapped by Tauri).

use crate::compute::day_of;
use crate::db::Db;
use crate::emitter::emit;
use crate::model::*;
use crate::strap::StrapManager;
use crate::{reports, screens};
use std::sync::Arc;
use tauri::{AppHandle, State};

pub struct AppState {
    pub db: Db,
    pub strap: Arc<StrapManager>,
}

type St<'a> = State<'a, Arc<AppState>>;
type R<T> = Result<T, String>;

async fn run<T: Send + 'static>(st: &St<'_>, f: impl FnOnce(&Db) -> R<T> + Send + 'static) -> R<T> {
    let db = st.db.clone();
    tokio::task::spawn_blocking(move || f(&db)).await.map_err(|e| e.to_string())?
}

fn announce(st: &St<'_>, days: Vec<String>) {
    emit(st.strap.emitter().as_ref(), "data://updated", &DataUpdatedEvent { days });
}

#[tauri::command]
pub async fn get_home(st: St<'_>, day: String) -> R<HomeData> {
    run(&st, move |db| screens::get_home(db, &day)).await
}
#[tauri::command]
pub async fn get_sleep(st: St<'_>, day: String) -> R<Avail<SleepData>> {
    run(&st, move |db| screens::get_sleep(db, &day)).await
}
#[tauri::command]
pub async fn get_recovery(st: St<'_>, day: String) -> R<Avail<RecoveryData>> {
    run(&st, move |db| screens::get_recovery(db, &day)).await
}
#[tauri::command]
pub async fn get_strain(st: St<'_>, day: String) -> R<Avail<StrainData>> {
    run(&st, move |db| screens::get_strain(db, &day)).await
}
#[tauri::command]
pub async fn get_activity(st: St<'_>, id: String) -> R<ActivityDetail> {
    run(&st, move |db| screens::get_activity(db, &id)).await
}
#[tauri::command]
pub async fn add_activity(st: St<'_>, kind: String, start_ms: i64, end_ms: i64) -> R<ActivitySummary> {
    let (a, day) = run(&st, move |db| screens::add_activity(db, &kind, start_ms, end_ms)).await?;
    announce(&st, vec![day]);
    Ok(a)
}
#[tauri::command]
pub async fn rename_activity(st: St<'_>, id: String, kind: String) -> R<ActivitySummary> {
    let kind = kind.trim().to_string();
    if kind.is_empty() {
        return Err("Give the activity a name.".into());
    }
    let (a, day) = run(&st, move |db| {
        db.rename_activity(&id, &kind)?;
        let a = db.activity(&id)?.ok_or("Unknown activity")?;
        let day = db.activity_day(&id).unwrap_or_else(|| day_of(a.start_ms));
        Ok((a, day))
    })
    .await?;
    announce(&st, vec![day]);
    Ok(a)
}
#[tauri::command]
pub async fn get_health(st: St<'_>, day: String) -> R<HealthData> {
    let live = st.strap.live_hr();
    run(&st, move |db| screens::get_health(db, &day, live)).await
}
#[tauri::command]
pub async fn get_metric_detail(st: St<'_>, metric: String, day: String) -> R<Avail<MetricDetail>> {
    run(&st, move |db| screens::get_metric_detail(db, &metric, &day)).await
}
#[tauri::command]
pub async fn get_stress(st: St<'_>, day: String) -> R<Avail<StressData>> {
    run(&st, move |db| screens::get_stress(db, &day)).await
}
#[tauri::command]
pub async fn get_trend(st: St<'_>, metric: String, range: String) -> R<Avail<TrendData>> {
    run(&st, move |db| reports::get_trend(db, &metric, &range)).await
}
#[tauri::command]
pub async fn get_journal(st: St<'_>, day: String) -> R<JournalDay> {
    run(&st, move |db| Ok(reports::get_journal(db, &day))).await
}
#[tauri::command]
pub async fn get_journal_library(st: St<'_>) -> R<Vec<JournalLibraryItem>> {
    run(&st, move |db| Ok(reports::get_journal_library(db))).await
}
#[tauri::command]
pub async fn set_journal_behaviors(st: St<'_>, ids: Vec<String>) -> R<()> {
    run(&st, move |db| db.set_journal_selected(&ids)).await?;
    announce(&st, vec![]);
    Ok(())
}
#[tauri::command]
pub async fn set_journal_answer(st: St<'_>, day: String, behavior: String, answer: Option<bool>) -> R<()> {
    let d = day.clone();
    run(&st, move |db| db.set_journal_answer(&d, &behavior, answer)).await?;
    announce(&st, vec![day]);
    Ok(())
}
#[tauri::command]
pub async fn get_insights(st: St<'_>) -> R<Vec<Insight>> {
    run(&st, move |db| Ok(reports::get_insights(db))).await
}
#[tauri::command]
pub async fn list_assessments(st: St<'_>) -> R<Vec<AssessmentListItem>> {
    run(&st, move |db| db.list_assessments()).await
}
#[tauri::command]
pub async fn get_assessment(st: St<'_>, id: String) -> R<Assessment> {
    run(&st, move |db| db.get_assessment(&id)).await
}
#[tauri::command]
pub async fn get_healthspan(st: St<'_>) -> R<Avail<HealthspanOut>> {
    run(&st, reports::get_healthspan).await
}
#[tauri::command]
pub async fn get_raw_day(st: St<'_>, day: String) -> R<RawDay> {
    run(&st, move |db| reports::get_raw_day(db, &day)).await
}
#[tauri::command]
pub async fn get_raw_night(st: St<'_>, day: String) -> R<Avail<RawNight>> {
    run(&st, move |db| reports::get_raw_night(db, &day)).await
}
#[tauri::command]
pub async fn get_raw_trends(st: St<'_>, range: String) -> R<RawTrends> {
    run(&st, move |db| reports::get_raw_trends(db, &range)).await
}
#[tauri::command]
pub async fn export_csv(st: St<'_>, kind: String, from: String, to: String) -> R<String> {
    run(&st, move |db| reports::export_csv(db, &kind, &from, &to)).await
}

// ------------------------------------------------------------------------- strap
#[tauri::command]
pub async fn get_strap_status(st: St<'_>) -> R<StrapStatus> {
    Ok(st.strap.status())
}
#[tauri::command]
pub async fn connect_strap(st: St<'_>, transport: String) -> R<()> {
    if transport != "ble" && transport != "emulator" {
        return Err(format!("Unknown transport '{transport}'"));
    }
    st.strap.connect(&transport).await
}
#[tauri::command]
pub async fn disconnect_strap(st: St<'_>) -> R<()> {
    st.strap.disconnect().await
}
#[tauri::command]
pub async fn sync_now(st: St<'_>) -> R<()> {
    let _nap = crate::nap::NapGuard::begin("Syncing the WHOOP strap");
    st.strap.sync_now().await
}
#[tauri::command]
pub async fn rebuild_decoded(st: St<'_>) -> R<()> {
    st.strap.rebuild().await
}
#[tauri::command]
pub async fn get_settings(st: St<'_>) -> R<Settings> {
    Ok(st.db.settings())
}
#[tauri::command]
pub async fn set_settings(app: AppHandle, st: St<'_>, settings: Settings) -> R<Settings> {
    let p = &settings.profile;
    if !(5.0..=120.0).contains(&p.age) || !(20.0..=400.0).contains(&p.weight_kg) || !(80.0..=260.0).contains(&p.height_cm) {
        return Err("Profile values are out of range (age 5-120, weight 20-400 kg, height 80-260 cm).".into());
    }
    if settings.transport != "ble" && settings.transport != "emulator" {
        return Err(format!("Unknown transport '{}'", settings.transport));
    }
    let before = st.db.settings();
    st.db.save_settings(&settings)?;
    crate::background::apply_autostart(&app, settings.launch_at_login);
    if before.profile != settings.profile {
        // the profile feeds max HR, calories and strain zones: refresh scores in the background
        let strap = st.strap.clone();
        tauri::async_runtime::spawn(async move {
            let _ = strap.rescore().await;
        });
    }
    Ok(st.db.settings())
}
#[tauri::command]
pub async fn emulator_control(st: St<'_>, action: String, value: Option<i64>) -> R<()> {
    st.strap.emulator_control(&action, value).await
}
