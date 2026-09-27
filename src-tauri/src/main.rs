#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use acs_core::{Agent, Draft, Session, Store, View, WindowState};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

struct State {
    session: Mutex<Option<Session>>,
    store: Store,
    window: Mutex<Option<WindowState>>,
}
fn failure(state: &State, stage: &str, error: impl std::fmt::Display) -> String {
    let text = format!("{error}");
    state.store.log(stage, &text);
    text
}

#[tauri::command]
fn load(state: tauri::State<State>) -> Result<View, String> {
    let mut guard = state
        .session
        .try_lock()
        .map_err(|_| "操作正在执行，请稍后重试".to_string())?;
    let session = Session::open(state.store.clone())
        .map_err(|e| failure(&state, "读取配置", format!("{e:#}")))?;
    let view = session.view();
    *guard = Some(session);
    Ok(view)
}
#[tauri::command]
fn save(
    state: tauri::State<State>,
    revision: String,
    draft: Draft,
    agent: Agent,
) -> Result<View, String> {
    let mut guard = state
        .session
        .try_lock()
        .map_err(|_| "操作正在执行".to_string())?;
    guard
        .as_mut()
        .ok_or("请先读取配置")?
        .save_for(&revision, &draft, agent)
        .map_err(|e| failure(&state, "保存配置", format!("{e:#}")))
}
#[tauri::command]
async fn launch(
    app: tauri::AppHandle,
    revision: String,
    draft: Draft,
    agent: String,
) -> Result<(), String> {
    let agent = agent.parse::<Agent>().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let guard = state
            .session
            .try_lock()
            .map_err(|_| "操作正在执行".to_string())?;
        let settings = guard
            .as_ref()
            .ok_or("请先读取配置")?
            .launch_settings_for(&revision, &draft, agent)
            .map_err(|e| failure(&state, "启动校验", format!("{e:#}")))?;
        acs_core::launch::launch_desktop(settings)
            .map_err(|e| failure(&state, "启动客户端", format!("{e:#}")))?;
        app.exit(0);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn repair(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<acs_core::history::Report, String> {
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let _guard = state
            .session
            .try_lock()
            .map_err(|_| "操作正在执行".to_string())?;
        acs_core::history::repair(&store, |progress| {
            let _ = app.emit("repair-progress", progress);
        })
        .map_err(|e| failure(&state, "修复对话", format!("{e:#}")))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn close(app: tauri::AppHandle) {
    app.exit(0);
}
#[tauri::command]
fn open_download(updates: bool, agent: String) -> Result<(), String> {
    let agent = agent.parse::<Agent>().map_err(|e| e.to_string())?;
    acs_core::launch::open_download(agent, updates).map_err(|e| format!("{e:#}"))
}

/// 显示器可能在两次打开之间被拔掉或重新排列；窗口中心不落在任何显示器上时
/// 放弃记住的位置，交给系统摆放，避免窗口开在看不见的地方。
fn on_visible_monitor(window: &tauri::Window, state: &WindowState) -> bool {
    let (x, y) = (state.x + state.width / 2.0, state.y + state.height / 2.0);
    window.available_monitors().is_ok_and(|monitors| {
        monitors.iter().any(|monitor| {
            let factor = monitor.scale_factor();
            let origin = monitor.position().to_logical::<f64>(factor);
            let size = monitor.size().to_logical::<f64>(factor);
            x >= origin.x
                && x < origin.x + size.width
                && y >= origin.y
                && y < origin.y + size.height
        })
    })
}
fn geometry(window: &tauri::Window, maximized: bool) -> Option<WindowState> {
    let factor = window.scale_factor().ok()?;
    let size = window.inner_size().ok()?.to_logical::<f64>(factor);
    let position = window.outer_position().ok()?.to_logical::<f64>(factor);
    (size.width > 0.0 && size.height > 0.0).then_some(WindowState {
        width: size.width,
        height: size.height,
        x: position.x,
        y: position.y,
        maximized,
    })
}
/// 最大化或最小化时的尺寸不是用户挑的尺寸，只更新最大化标记，
/// 保留上一次手动调整的几何作为还原尺寸。
fn remember(window: &tauri::Window) {
    let state = window.state::<State>();
    let maximized = window.is_maximized().unwrap_or(false);
    let mut guard = state.window.lock().unwrap();
    if maximized || window.is_minimized().unwrap_or(false) {
        if let Some(previous) = guard.as_mut() {
            previous.maximized = maximized;
        }
        return;
    }
    if let Some(current) = geometry(window, maximized) {
        *guard = Some(current);
    }
}

fn main() {
    let store = Store::discover().expect("无法确定配置目录");
    let app = tauri::Builder::default()
        .manage(State {
            session: Mutex::new(None),
            store,
            window: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            load,
            save,
            launch,
            repair,
            close,
            open_download
        ])
        .setup(|app| {
            let window = app
                .get_webview_window("main")
                .expect("缺少主窗口")
                .as_ref()
                .window();
            let state = app.state::<State>();
            let saved = state.store.window_state();
            if let Some(saved) = saved {
                let _ = window.set_size(tauri::LogicalSize::new(saved.width, saved.height));
                if on_visible_monitor(&window, &saved) {
                    let _ = window.set_position(tauri::LogicalPosition::new(saved.x, saved.y));
                }
                if saved.maximized {
                    let _ = window.maximize();
                }
            }
            // 记住的几何本身就是还原目标；只有首次运行才需要读取窗口当前位置。
            *state.window.lock().unwrap() = saved.or_else(|| geometry(&window, false));
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.emit("request-close", ());
            }
            tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_) => remember(window),
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("桌面窗口启动失败");
    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            let state = app.state::<State>();
            let current = *state.window.lock().unwrap();
            if let Some(current) = current {
                state.store.set_window_state(&current);
            }
        }
    });
}
