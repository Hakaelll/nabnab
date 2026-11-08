use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Webview, WindowBuilder, WindowEvent,
};

const MAIN: &str = "main";
const MINI: &str = "playback-mini";
const MINI_WIDTH: f64 = 380.;

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    DetachAudio,
    DetachVideo,
    Restore,
    Drag,
    ResizeAudio,
    ResizeVideo,
}

fn mini_height(video: bool) -> f64 {
    if video {
        300.
    } else {
        190.
    }
}

fn resize(mini: &tauri::Window, video: bool) -> tauri::Result<()> {
    let position = mini.outer_position()?;
    let bottom = position.y + mini.outer_size()?.height as i32;
    mini.set_size(LogicalSize::new(MINI_WIDTH, mini_height(video)))?;
    let top = mini
        .current_monitor()?
        .map_or(i32::MIN, |monitor| monitor.work_area().position.y);
    mini.set_position(PhysicalPosition::new(
        position.x,
        (bottom - mini.outer_size()?.height as i32).max(top),
    ))
}

fn detach(app: &AppHandle, video: bool) -> tauri::Result<()> {
    let Some(main) = app.get_window(MAIN) else {
        return Err(tauri::Error::WindowNotFound);
    };
    let Some(view) = app.get_webview(MAIN) else {
        return Err(tauri::Error::WebviewNotFound);
    };
    if view.window().label() == MINI {
        return Ok(());
    }
    let mini = WindowBuilder::new(app, MINI)
        .title(if video {
            "nabnab · Video"
        } else {
            "nabnab · Audio"
        })
        .inner_size(MINI_WIDTH, mini_height(video))
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    let place = || {
        if let Some(monitor) = main.current_monitor()? {
            let area = monitor.work_area();
            let size = mini.outer_size()?;
            let margin = (16. * monitor.scale_factor()).round() as i32;
            mini.set_position(PhysicalPosition::new(
                area.position.x
                    + (area.size.width.saturating_sub(size.width) as i32 - margin).max(0),
                area.position.y
                    + (area.size.height.saturating_sub(size.height) as i32 - margin).max(0),
            ))?;
        }
        Ok(())
    };
    if let Err(error) = place().and_then(|()| view.reparent(&mini)) {
        let _ = mini.destroy();
        return Err(error);
    }
    if let Err(error) = main
        .minimize()
        .and_then(|()| mini.show())
        .and_then(|()| mini.set_focus())
    {
        let _ = restore(app);
        return Err(error);
    }
    Ok(())
}

fn restore(app: &AppHandle) -> tauri::Result<()> {
    let Some(view) = app.get_webview(MAIN) else {
        return Err(tauri::Error::WebviewNotFound);
    };
    let mini = view.window();
    if mini.label() != MINI {
        return Ok(());
    }
    let Some(main) = app.get_window(MAIN) else {
        return Err(tauri::Error::WindowNotFound);
    };
    main.show()?;
    main.unminimize()?;
    view.reparent(&main)?;
    mini.destroy()?;
    main.set_focus()?;
    app.emit("playback-mini-restored", ())?;
    Ok(())
}

#[tauri::command]
pub async fn playback_mini_window(
    app: AppHandle,
    webview: Webview,
    action: Action,
) -> Result<(), String> {
    if webview.label() != MAIN {
        return Err("Only the nabnab playback view can control this window.".into());
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let task = app.clone();
    app.run_on_main_thread(move || {
        let host = webview.window();
        let result = match action {
            Action::DetachAudio | Action::DetachVideo => {
                detach(&task, matches!(action, Action::DetachVideo))
            }
            Action::Restore => restore(&task),
            Action::Drag if host.label() == MINI => host.start_dragging(),
            Action::ResizeAudio | Action::ResizeVideo if host.label() == MINI => {
                resize(&host, matches!(action, Action::ResizeVideo))
            }
            _ => Ok(()),
        };
        let _ = send.send(result.map_err(|error| error.to_string()));
    })
    .map_err(|error| error.to_string())?;
    receive
        .await
        .map_err(|_| "The playback window did not respond.".to_string())?
}

pub fn window_event(app: &AppHandle, label: &str, event: &WindowEvent) {
    match (label, event) {
        (MINI, WindowEvent::CloseRequested { api, .. }) => {
            api.prevent_close();
            let app = app.clone();
            let _ = app.clone().run_on_main_thread(move || {
                let _ = restore(&app);
            });
        }
        (MAIN, WindowEvent::Focused(true)) if app.get_window(MINI).is_some() => {
            let app = app.clone();
            let _ = app.clone().run_on_main_thread(move || {
                let _ = restore(&app);
            });
        }
        (MAIN, WindowEvent::Destroyed) => {
            if let Some(mini) = app.get_window(MINI) {
                let _ = mini.destroy();
            }
        }
        _ => {}
    }
}
