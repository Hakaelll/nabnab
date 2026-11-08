use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, Webview};

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{shutdown, DesktopWheel};

#[cfg(not(windows))]
#[derive(Default)]
pub struct DesktopWheel;

#[cfg(not(windows))]
pub fn shutdown(_: &AppHandle) {}

const OVERLAY: &str = "quick-skip";

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Action {
    Ready,
    Hide { gesture: u32 },
    Transport { gesture: u32, action: Value },
}

#[tauri::command]
pub async fn quick_skip_session(
    app: AppHandle,
    webview: Webview,
    configuration: Option<Value>,
    revision: u32,
) -> Result<(), String> {
    if webview.label() != "main" {
        return Err("Only the playback session can configure Quick Skip.".into());
    }
    #[cfg(windows)]
    {
        windows::on_main(app, move |app| {
            app.state::<DesktopWheel>()
                .configure(app, configuration, revision)
        })
        .await
    }
    #[cfg(not(windows))]
    {
        let _ = (app, configuration, revision);
        Ok(())
    }
}

#[tauri::command]
pub async fn quick_skip_overlay(
    app: AppHandle,
    webview: Webview,
    action: Action,
) -> Result<(), String> {
    if webview.label() != OVERLAY {
        return Err("Only the Quick Skip window can send overlay actions.".into());
    }
    #[cfg(windows)]
    {
        windows::on_main(app, move |app| {
            app.state::<DesktopWheel>().action(app, action)
        })
        .await
    }
    #[cfg(not(windows))]
    {
        let _ = (app, action);
        Ok(())
    }
}
