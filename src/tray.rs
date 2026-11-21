use futures::channel::mpsc::UnboundedSender;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{MenuItem, StandardItem};

use crate::AppEvent;

pub struct AppTray {
    events: UnboundedSender<AppEvent>,
    pub tooltip: String,
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
        let _ = self.events.unbounded_send(AppEvent::ShowWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Показать окно".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.events.unbounded_send(AppEvent::ShowWindow);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Выход (вернуть весь звук)".into(),
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
    match (AppTray { events, tooltip: String::new() }).spawn() {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("tray unavailable: {e}");
            None
        }
    }
}
