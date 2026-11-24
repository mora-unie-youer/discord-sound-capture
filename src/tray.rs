use futures::channel::mpsc::UnboundedSender;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{MenuItem, StandardItem};

use crate::AppEvent;

pub struct AppTray {
    events: UnboundedSender<AppEvent>,
    pub tooltip: String,
    pub window_visible: bool,
}

impl ksni::Tray for AppTray {
    fn id(&self) -> String {
        "discord-sound-capture".into()
    }

    fn title(&self) -> String {
        "Discord Sound Capture".into()
    }

    fn icon_name(&self) -> String {
        "audio-volume-high".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Discord Sound Capture".into(),
            description: self.tooltip.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.events.unbounded_send(AppEvent::ToggleWindow);
    }

    fn watcher_online(&self) {
        let _ = self.events.unbounded_send(AppEvent::TrayOnline);
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        let _ = self.events.unbounded_send(AppEvent::TrayOffline);
        true
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: if self.window_visible { "Hide window" } else { "Show window" }.into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.events.unbounded_send(AppEvent::ToggleWindow);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit (restore all audio)".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.events.unbounded_send(AppEvent::QuitRequested);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub fn spawn(events: UnboundedSender<AppEvent>) -> Option<Handle<AppTray>> {
    // At autostart the StatusNotifierWatcher may not be on the bus yet. With
    // `assume_sni_available` ksni waits for it and registers once it appears
    // instead of failing (which left a hidden, trayless process).
    match (AppTray { events, tooltip: String::new(), window_visible: false })
        .assume_sni_available(true)
        .spawn()
    {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("tray unavailable: {e}");
            None
        }
    }
}
