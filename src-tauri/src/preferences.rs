use crate::store::Settings;
use std::path::Path;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

const TRAY_ID: &str = "nabnab";

fn show_main(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("The main window is unavailable.")?;
    window
        .unminimize()
        .map_err(|_| "Could not restore the nabnab window.")?;
    window
        .show()
        .map_err(|_| "Could not show the nabnab window.")?;
    window
        .set_focus()
        .map_err(|_| "Could not focus the nabnab window.".into())
}

fn set_tray(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        return tray
            .set_visible(enabled)
            .map_err(|_| "Could not change the nabnab tray icon.".into());
    }
    if !enabled {
        return Ok(());
    }
    let show = MenuItem::with_id(app, "nabnab-show", "Mostrar nabnab", true, None::<&str>)
        .map_err(|_| "Could not prepare the tray menu.")?;
    let exit = MenuItem::with_id(app, "nabnab-exit", "Salir", true, None::<&str>)
        .map_err(|_| "Could not prepare the tray menu.")?;
    let menu =
        Menu::with_items(app, &[&show, &exit]).map_err(|_| "Could not prepare the tray menu.")?;
    let icon = app
        .default_window_icon()
        .ok_or("The nabnab application icon is unavailable.")?
        .clone();
    TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("nabnab")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "nabnab-show" => {
                let _ = show_main(app);
            }
            "nabnab-exit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                let _ = show_main(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|_| "Could not add the nabnab tray icon.")?;
    Ok(())
}

fn startup_command(executable: &Path) -> Result<String, String> {
    let text = executable
        .to_str()
        .ok_or("The application path cannot be registered for Windows startup.")?;
    if !executable.is_absolute()
        || executable
            .extension()
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("exe"))
        || text.chars().any(|c| matches!(c, '"' | '\0' | '\r' | '\n'))
    {
        return Err("The application path cannot be registered for Windows startup.".into());
    }
    Ok(format!("\"{text}\" --startup"))
}

#[cfg(windows)]
fn set_windows_startup(enabled: bool) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
            RegSetValueExW, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE,
            REG_OPTION_NON_VOLATILE, REG_SZ,
        },
    };
    let executable =
        std::env::current_exe().map_err(|_| "Could not find the nabnab executable.")?;
    let command = startup_command(&executable)?;
    // Debug UI verification never changes the real user's login items.
    if crate::test_data_dir().is_some() {
        return Ok(());
    }
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let value: Vec<u16> = "nabnab".encode_utf16().chain(Some(0)).collect();
    let mut key = std::ptr::null_mut();
    let opened = unsafe {
        if enabled {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        } else {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                &mut key,
            )
        }
    };
    if opened == ERROR_FILE_NOT_FOUND && !enabled {
        return Ok(());
    }
    if opened != ERROR_SUCCESS {
        return Err(
            "Windows could not change nabnab's startup preference. Check your account permissions."
                .into(),
        );
    }
    let result = (|| {
        let mut kind = 0;
        let mut length = 0;
        let found = unsafe {
            RegQueryValueExW(
                key,
                value.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut length,
            )
        };
        let existing = if found == ERROR_FILE_NOT_FOUND {
            None
        } else if found == ERROR_SUCCESS && kind == REG_SZ && length <= 64 * 1024 && length % 2 == 0
        {
            let mut content = vec![0u16; length as usize / 2];
            let read = unsafe {
                RegQueryValueExW(
                    key,
                    value.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    content.as_mut_ptr().cast(),
                    &mut length,
                )
            };
            if read != ERROR_SUCCESS {
                return Err("Windows could not read nabnab's startup preference.".into());
            }
            while content.last() == Some(&0) {
                content.pop();
            }
            Some(
                String::from_utf16(&content)
                    .map_err(|_| "The existing Windows startup entry is invalid.")?,
            )
        } else if !enabled {
            return Ok(());
        } else {
            return Err("An incompatible Windows startup entry already uses the name nabnab. Resolve it in Windows Startup Apps before enabling this option.".into());
        };
        if enabled {
            if existing
                .as_ref()
                .is_some_and(|existing| existing != &command)
            {
                return Err("Another Windows startup entry uses the name nabnab. Resolve it in Windows Startup Apps before enabling this option.".into());
            }
            let command: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
            if unsafe {
                RegSetValueExW(
                    key,
                    value.as_ptr(),
                    0,
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            } != ERROR_SUCCESS
            {
                return Err("Windows could not enable nabnab at login.".into());
            }
        } else if existing.as_deref() == Some(command.as_str()) {
            let deleted = unsafe { RegDeleteValueW(key, value.as_ptr()) };
            if !matches!(deleted, ERROR_SUCCESS | ERROR_FILE_NOT_FOUND) {
                return Err("Windows could not disable nabnab at login.".into());
            }
        }
        Ok(())
    })();
    unsafe {
        RegCloseKey(key);
    }
    result
}

#[cfg(not(windows))]
fn set_windows_startup(enabled: bool) -> Result<(), String> {
    if enabled {
        Err("Starting with Windows is available in the Windows build.".into())
    } else {
        Ok(())
    }
}

pub fn apply(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    settings.validate()?;
    set_windows_startup(settings.start_with_windows)?;
    set_tray(app, settings.tray_enabled)
}

pub fn close_to_tray(app: &AppHandle) -> Result<(), String> {
    if app.tray_by_id(TRAY_ID).is_none() {
        return Err("The tray icon is unavailable. The window was kept open.".into());
    }
    app.get_webview_window("main")
        .ok_or("The main window is unavailable.")?
        .hide()
        .map_err(|_| "Could not hide nabnab in the tray.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_registration_quotes_the_whole_executable_and_uses_one_fixed_argument() {
        let executable = std::env::temp_dir()
            .join("nabnab folder & files")
            .join("nabnab.exe");
        assert_eq!(
            startup_command(&executable).unwrap(),
            format!("\"{}\" --startup", executable.display())
        );
        assert!(startup_command(Path::new("relative.exe")).is_err());
        assert!(startup_command(&std::env::temp_dir().join("not-executable.txt")).is_err());
        for value in [
            "invalid\"name.exe",
            "invalid\nname.exe",
            "invalid\rname.exe",
            "invalid\0name.exe",
        ] {
            assert!(startup_command(&std::env::temp_dir().join(value)).is_err());
        }
    }
}
