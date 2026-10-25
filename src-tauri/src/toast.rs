//! Windows notifications, like v1's: a map in the watched folder was saved, click to start it
//! (and: the server is ready, click to join). They're shown while DropLaunch is in the background
//! (the UI decides), and a click comes back to this window as a "notification-clicked" event.
//!
//! Windows shows notifications under an app id (AppUserModelID). An app without an installer
//! registers its name and icon for that id under `HKCU\Software\Classes\AppUserModelId`; no Start
//! menu shortcut needed.

use serde::{Deserialize, Serialize};

/// Same as the app identifier in tauri.conf.json.
pub const APP_ID: &str = "gg.droplaunch.desktop";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Toast {
    /// A newer notification with the same tag replaces the older one (e.g. "saved").
    pub tag: String,
    pub title: String,
    pub body: String,
    /// What a click on the notification itself does (sent back with the click).
    pub action: String,
    /// Buttons: (label, action).
    #[serde(default)]
    pub buttons: Vec<(String, String)>,
}

/// The "notification-clicked" event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clicked {
    pub tag: String,
    pub action: String,
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The notification's XML (Windows' toast schema).
fn xml(toast: &Toast, icon: Option<&str>) -> String {
    let logo = icon.map_or(String::new(), |path| format!(r#"<image placement="appLogoOverride" src="{}"/>"#, escape(path)));
    let buttons: String = toast
        .buttons
        .iter()
        .map(|(label, action)| format!(r#"<action content="{}" arguments="{}" activationType="foreground"/>"#, escape(label), escape(action)))
        .collect();
    let actions = if buttons.is_empty() { String::new() } else { format!("<actions>{buttons}</actions>") };
    format!(
        r#"<toast launch="{}" activationType="foreground"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text>{logo}</binding></visual>{actions}</toast>"#,
        escape(&toast.action),
        escape(&toast.title),
        escape(&toast.body),
    )
}

#[cfg(windows)]
mod imp {
    use super::{xml, Clicked, Toast, APP_ID};
    use std::path::PathBuf;
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Mutex, OnceLock};
    use tauri::{AppHandle, Emitter, Manager};
    use windows::core::{IInspectable, Interface, HSTRING};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager};

    /// Notifications are made on one thread of their own (WinRT wants COM set up on it).
    static WORKER: OnceLock<Mutex<Sender<Toast>>> = OnceLock::new();
    /// Shown notifications are kept, or their click handlers would go with them.
    const KEEP: usize = 8;

    pub fn show(app: &AppHandle, toast: Toast) -> Result<(), String> {
        let worker = WORKER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Toast>();
            let app = app.clone();
            std::thread::Builder::new()
                .name("notifications".into())
                .spawn(move || {
                    // SAFETY: called once on this thread before any WinRT use.
                    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
                    let icon = register();
                    let mut shown: Vec<ToastNotification> = Vec::new();
                    for toast in rx {
                        match make(&app, &toast, icon.as_deref()) {
                            Ok(n) => {
                                shown.push(n);
                                if shown.len() > KEEP {
                                    shown.remove(0);
                                }
                            }
                            Err(e) => eprintln!("notification failed: {e}"),
                        }
                    }
                })
                .expect("spawn notifications thread");
            Mutex::new(tx)
        });
        worker.lock().unwrap().send(toast).map_err(|_| crate::t!("notify.error.unavailable"))
    }

    fn make(app: &AppHandle, toast: &Toast, icon: Option<&str>) -> windows::core::Result<ToastNotification> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml(toast, icon)))?;
        let notification = ToastNotification::CreateToastNotification(&doc)?;
        notification.SetTag(&HSTRING::from(&toast.tag))?;
        let (app, tag) = (app.clone(), toast.tag.clone());
        notification.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, args| {
            let action = args.as_ref().and_then(|a| a.cast::<ToastActivatedEventArgs>().ok()).and_then(|a| a.Arguments().ok()).map(|a| a.to_string()).unwrap_or_default();
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            let _ = app.emit("notification-clicked", Clicked { tag: tag.clone(), action });
            Ok(())
        }))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&notification)?;
        Ok(notification)
    }

    /// DropLaunch's name and icon for its notifications. Returns the icon's path.
    fn register() -> Option<String> {
        let key = format!(r"Software\Classes\AppUserModelId\{APP_ID}");
        let _ = crate::platform::set_user_registry_string(&key, "DisplayName", "DropLaunch");
        let dir = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?.join("DropLaunch");
        let icon = dir.join("notification-icon.png");
        let bytes = include_bytes!("../icons/icon.png");
        if std::fs::metadata(&icon).map_or(true, |m| m.len() != bytes.len() as u64) {
            std::fs::create_dir_all(&dir).ok()?;
            std::fs::write(&icon, bytes).ok()?;
        }
        let path = icon.display().to_string();
        let _ = crate::platform::set_user_registry_string(&key, "IconUri", &path);
        Some(path)
    }
}

#[cfg(windows)]
pub use imp::show;

#[cfg(not(windows))]
pub fn show(_app: &tauri::AppHandle, _toast: Toast) -> Result<(), String> {
    Err(crate::t!("notify.error.windowsOnly"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_is_escaped() {
        let toast = Toast {
            tag: "saved".into(),
            title: "Rock & <Roll> \"v2\" was saved".into(),
            body: "Click to start it".into(),
            action: "start".into(),
            buttons: vec![("Prefab test mode".into(), "prefab".into())],
        };
        let x = xml(&toast, Some(r"C:\Users\A&B\icon.png"));
        assert!(x.contains("Rock &amp; &lt;Roll&gt; &quot;v2&quot; was saved"));
        assert!(x.contains(r#"launch="start""#));
        assert!(x.contains(r#"<action content="Prefab test mode" arguments="prefab""#));
        assert!(x.contains(r"A&amp;B\icon.png"));
    }
}
