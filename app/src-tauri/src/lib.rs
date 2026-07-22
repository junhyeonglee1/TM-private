#[cfg(all(not(debug_assertions), not(feature = "custom-protocol")))]
compile_error!("TM release builds require the `custom-protocol` feature");

mod cloud_client;
mod commands;

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use serde_json::Value;
use tauri::Manager;
use tm_core::{TmCore, TmHome};

use cloud_client::{CloudClient, DataMode};

pub(crate) struct AppState {
    core: TmCore,
    cloud: CloudClient,
    shutdown_backed_up: AtomicBool,
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
fn data_mode(state: tauri::State<'_, AppState>) -> &'static str {
    state.cloud.mode().as_str()
}

#[tauri::command]
async fn invoke_cloud_command(
    command: String,
    args: Option<Value>,
    state: tauri::State<'_, AppState>,
) -> Result<Value, String> {
    let cloud = state.cloud.clone();
    cloud
        .invoke(
            &command,
            args.unwrap_or_else(|| Value::Object(serde_json::Map::new())),
        )
        .await
}

#[tauri::command]
async fn invoke_cloud_device_admin(
    command: String,
    args: Option<Value>,
    state: tauri::State<'_, AppState>,
) -> Result<Value, String> {
    let cloud = state.cloud.clone();
    cloud
        .device_admin(
            &command,
            args.unwrap_or_else(|| Value::Object(serde_json::Map::new())),
        )
        .await
}

#[tauri::command]
async fn invoke_cloud_assistant_feature(
    command: String,
    args: Option<Value>,
    state: tauri::State<'_, AppState>,
) -> Result<Value, String> {
    let cloud = state.cloud.clone();
    cloud
        .assistant_feature(
            &command,
            args.unwrap_or_else(|| Value::Object(serde_json::Map::new())),
        )
        .await
}

#[tauri::command]
async fn get_cost_status(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let cloud = state.cloud.clone();
    cloud.cost_status().await
}

pub fn run() {
    let application = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let home = TmHome::from_env();
            home.ensure_layout()?;
            let cloud = CloudClient::load(&home).map_err(std::io::Error::other)?;
            let data_mode = cloud.mode();
            let file_appender = tracing_appender::rolling::daily(home.logs_dir(), "tm.log");
            let _ = tracing_subscriber::fmt()
                .with_writer(file_appender)
                .with_ansi(false)
                .try_init();

            // Cloud mode must never open the archived local authority database in
            // read-write/WAL mode. Keep legacy IPC commands isolated in an unused
            // scratch home while the UI routes every command through HTTPS.
            let core_home = if data_mode == DataMode::Cloud {
                TmHome::new(home.dist_dir().join("cloud-local-disabled"))
            } else {
                home
            };
            let core = TmCore::open(core_home)?;
            if data_mode == DataMode::Local {
                core.create_startup_backup()?;
                let daily_core = core.clone();
                std::thread::spawn(move || {
                    loop {
                        std::thread::sleep(Duration::from_secs(15 * 60));
                        if let Err(error) = daily_core.create_daily_backup_if_due() {
                            tracing::error!(%error, "서울 날짜 기준 일일 백업에 실패했습니다");
                        }
                    }
                });
            }
            app.manage(AppState {
                core,
                cloud,
                shutdown_backed_up: AtomicBool::new(false),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            data_mode,
            invoke_cloud_command,
            invoke_cloud_device_admin,
            invoke_cloud_assistant_feature,
            get_cost_status,
            commands::get_app_snapshot,
            commands::get_calendar_month,
            commands::create_calendar_event,
            commands::update_calendar_event,
            commands::delete_calendar_event,
            commands::create_project,
            commands::create_task,
            commands::update_task,
            commands::plan_task,
            commands::resolve_day_entry,
            commands::start_session,
            commands::finish_session,
            commands::create_work_log,
            commands::create_note,
            commands::create_change_request,
            commands::update_change_request,
            commands::approve_change_request,
            commands::return_change_request_to_draft,
            commands::cancel_change_request,
            commands::abandon_change_request,
            commands::promote_work_log,
            commands::search,
            commands::move_to_trash,
            commands::restore_trash_item,
            commands::create_backup,
            commands::restore_backup,
            commands::export_all,
        ])
        .build(tauri::generate_context!())
        .expect("TM 데스크톱 런타임을 구성하지 못했습니다");

    application.run(|app_handle, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            let state = app_handle.state::<AppState>();
            if state.cloud.mode() == DataMode::Cloud {
                return;
            }
            if !state.shutdown_backed_up.load(Ordering::SeqCst) {
                match state.core.create_shutdown_backup() {
                    Ok(_) => state.shutdown_backed_up.store(true, Ordering::SeqCst),
                    Err(error) => {
                        tracing::error!(%error, "종료 백업에 실패하여 앱 종료를 중단했습니다");
                        api.prevent_exit();
                    }
                }
            }
        }
    });
}
