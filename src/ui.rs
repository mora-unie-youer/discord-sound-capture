use std::collections::HashMap;

use iced::widget::{button, checkbox, column, container, pick_list, row, scrollable, space, text, text_input};
use iced::{Element, Fill, Size, Subscription, Task, window};

use crate::config::{self, Config, LoadOutcome};
use crate::pulse::{CaptureStatus, Command, CoreHandle, Snapshot, StreamView};
use crate::rules::{Field, Rule, validate_pattern};
use crate::wpcheck;
use crate::{AppEvent, INITIAL_RULES, TrayHandle, worker};

#[derive(Default)]
struct Banners {
    config_error: Option<String>,
    save_error: Option<String>,
    wp_rule_missing: bool,
    core_died: bool,
}

pub struct App {
    core: Option<CoreHandle>,
    tray: Option<TrayHandle>,
    tray_available: bool,
    window: Option<window::Id>,
    config: Config,
    snapshot: Snapshot,
    banners: Banners,
    quitting: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Event(AppEvent),
    WindowOpened(window::Id),
    WindowCloseRequested(window::Id),
    ToggleBinary { binary: String, label: Option<String>, on: bool },
    AddRegex,
    RegexField(usize, Field),
    RegexPattern(usize, String),
    RemoveRule(usize),
    CopyWpRule,
    HideToTray,
    Quit,
}

fn window_settings() -> window::Settings {
    window::Settings {
        size: Size::new(720.0, 640.0),
        exit_on_close_request: false,
        ..Default::default()
    }
}

impl App {
    pub fn boot() -> (Self, Task<Message>) {
        let mut banners = Banners::default();
        let config = match config::load() {
            LoadOutcome::Loaded(c) => c,
            LoadOutcome::Missing => Config::default(),
            LoadOutcome::Broken { moved_to, error } => {
                banners.config_error = Some(format!(
                    "Конфиг повреждён ({error}), сохранён как {}",
                    moved_to.display()
                ));
                Config::default()
            }
        };
        banners.wp_rule_missing = !wpcheck::wp_rule_present();
        let _ = INITIAL_RULES.set(config.rules.clone());

        let hidden = std::env::args().any(|a| a == "--hidden");
        let mut app = App {
            core: None,
            tray: None,
            tray_available: true,
            window: None,
            config,
            snapshot: Snapshot::default(),
            banners,
            quitting: false,
        };
        let task = if hidden {
            Task::none()
        } else {
            app.open_window()
        };
        (app, task)
    }

    fn open_window(&mut self) -> Task<Message> {
        let (id, task) = window::open(window_settings());
        self.window = Some(id);
        task.map(Message::WindowOpened)
    }

    fn persist(&mut self) {
        match config::save(&self.config) {
            Ok(()) => self.banners.save_error = None,
            Err(e) => self.banners.save_error = Some(format!("Не удалось сохранить конфиг: {e}")),
        }
        if let Some(core) = &self.core {
            core.send(Command::SetRules(self.config.rules.clone()));
        }
    }

    fn begin_quit(&mut self) -> Task<Message> {
        if self.quitting {
            return Task::none();
        }
        self.quitting = true;
        match &self.core {
            Some(core) => {
                core.send(Command::Shutdown);
                Task::none()
            }
            None => iced::exit(),
        }
    }

    fn close_window(&mut self, id: window::Id) -> Task<Message> {
        if self.tray_available {
            self.window = None;
            window::close(id)
        } else {
            self.begin_quit()
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Event(ev) => match ev {
                AppEvent::Ready { core, tray } => {
                    self.core = Some(core);
                    self.tray_available = tray.is_some();
                    self.tray = tray;
                    Task::none()
                }
                AppEvent::Snapshot(s) => {
                    let count = |c| s.streams.iter().filter(|v| v.capture == c).count();
                    let (inn, blocked) = (count(CaptureStatus::InStream), count(CaptureStatus::Blocked));
                    self.snapshot = s;
                    if let Some(t) = &self.tray {
                        t.0.update(|t| t.tooltip = format!("В стриме: {inn}, заглушено: {blocked}"));
                    }
                    Task::none()
                }
                AppEvent::ShowWindow => match self.window {
                    Some(id) => window::gain_focus(id),
                    None => self.open_window(),
                },
                AppEvent::QuitRequested => self.begin_quit(),
                AppEvent::ShutdownComplete => iced::exit(),
                AppEvent::CoreDied => {
                    self.banners.core_died = true;
                    self.core = None;
                    if self.quitting { iced::exit() } else { Task::none() }
                }
            },
            Message::WindowOpened(id) => {
                self.window = Some(id);
                Task::none()
            }
            Message::WindowCloseRequested(id) => self.close_window(id),
            Message::HideToTray => match self.window {
                Some(id) if self.tray_available => self.close_window(id),
                _ => Task::none(),
            },
            Message::Quit => self.begin_quit(),
            Message::ToggleBinary { binary, label, on } => {
                if on {
                    let exists = self
                        .config
                        .rules
                        .iter()
                        .any(|r| matches!(r, Rule::Binary { binary: b, .. } if *b == binary));
                    if !exists {
                        self.config.rules.push(Rule::Binary { binary, label });
                    }
                } else {
                    self.config
                        .rules
                        .retain(|r| !matches!(r, Rule::Binary { binary: b, .. } if *b == binary));
                }
                self.persist();
                Task::none()
            }
            Message::AddRegex => {
                self.config
                    .rules
                    .push(Rule::Regex { field: Field::ApplicationName, pattern: String::new() });
                self.persist();
                Task::none()
            }
            Message::RegexField(i, f) => {
                if let Some(Rule::Regex { field, .. }) = self.config.rules.get_mut(i) {
                    *field = f;
                    self.persist();
                }
                Task::none()
            }
            Message::RegexPattern(i, p) => {
                if let Some(Rule::Regex { pattern, .. }) = self.config.rules.get_mut(i) {
                    *pattern = p;
                    self.persist();
                }
                Task::none()
            }
            Message::RemoveRule(i) => {
                if matches!(self.config.rules.get(i), Some(Rule::Regex { .. })) {
                    self.config.rules.remove(i);
                    self.persist();
                }
                Task::none()
            }
            Message::CopyWpRule => iced::clipboard::write(wpcheck::WP_RULE_TEXT.to_string()),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(worker).map(Message::Event),
            window::close_requests().map(Message::WindowCloseRequested),
        ])
    }

    pub fn view(&self, _id: window::Id) -> Element<'_, Message> {
        let mut col = column![].spacing(12);

        for b in self.banner_views() {
            col = col.push(b);
        }

        col = col.push(text("Приложения").size(20));
        for r in self.app_rows() {
            col = col.push(r);
        }

        col = col.push(text("Regex-правила").size(20));
        for (i, rule) in self.config.rules.iter().enumerate() {
            if let Rule::Regex { field, pattern } = rule {
                col = col.push(self.regex_row(i, field, pattern));
            }
        }
        col = col.push(button("+ Добавить regex").on_press(Message::AddRegex));
        col = col.push(
            text("Совпадение по подстроке; используйте ^…$ для точного. Поток разрешён, если совпало любое правило.")
                .size(12),
        );

        if self.quitting {
            col = col.push(text("Завершение: возвращаю звук…"));
        } else {
            let mut footer = row![].spacing(8);
            if self.tray_available {
                footer = footer.push(button("Скрыть в трей").on_press(Message::HideToTray));
            }
            footer = footer.push(
                button("Выход (вернуть весь звук)")
                    .on_press(Message::Quit)
                    .style(button::danger),
            );
            col = col.push(footer);
        }

        scrollable(container(col).padding(16)).width(Fill).height(Fill).into()
    }

    fn banner_views(&self) -> Vec<Element<'_, Message>> {
        fn banner<'a>(msg: String, warn: bool) -> Element<'a, Message> {
            let t = text(msg);
            let t = if warn { t.style(text::warning) } else { t.style(text::danger) };
            container(t).padding(8).width(Fill).style(container::bordered_box).into()
        }
        let b = &self.banners;
        let mut v: Vec<Element<'_, Message>> = vec![];
        if b.wp_rule_missing {
            v.push(
                container(
                    column![
                        text(format!(
                            "Правило WirePlumber не найдено: заглушённое состояние может сохраниться после выхода. Создайте {} и перезапустите стек PipeWire.",
                            wpcheck::WP_RULE_PATH_HINT
                        ))
                        .style(text::warning),
                        button("Скопировать правило").on_press(Message::CopyWpRule),
                    ]
                    .spacing(8),
                )
                .padding(8)
                .width(Fill)
                .style(container::bordered_box)
                .into(),
            );
        }
        if !self.snapshot.connected {
            v.push(banner("Нет соединения с PulseAudio, переподключение…".into(), true));
        }
        if b.core_died {
            v.push(banner("Ядро остановилось. Перезапустите приложение.".into(), false));
        }
        if let Some(e) = &b.config_error {
            v.push(banner(e.clone(), false));
        }
        if let Some(e) = &b.save_error {
            v.push(banner(e.clone(), false));
        }
        if !self.tray_available {
            v.push(banner(
                "Трей недоступен: закрытие окна завершит программу и вернёт весь звук.".into(),
                true,
            ));
        }
        v
    }

    fn app_rows(&self) -> Vec<Element<'_, Message>> {
        struct Row<'a> {
            binary: &'a str,
            label: Option<&'a str>,
            checked: bool,
            streams: Vec<&'a StreamView>,
        }
        let mut groups: HashMap<&str, Row<'_>> = HashMap::new();
        let mut order: Vec<&str> = vec![];
        for v in &self.snapshot.streams {
            let Some(b) = v.info.binary.as_deref() else { continue };
            let row = groups.entry(b).or_insert_with(|| {
                order.push(b);
                Row {
                    binary: b,
                    label: None,
                    checked: self
                        .config
                        .rules
                        .iter()
                        .any(|r| matches!(r, Rule::Binary { binary, .. } if binary == b)),
                    streams: vec![],
                }
            });
            if row.label.is_none() {
                row.label = v.info.app_name.as_deref().filter(|s| !s.is_empty());
            }
            row.streams.push(v);
        }
        let mut rows: Vec<Row<'_>> = order.into_iter().filter_map(|b| groups.remove(b)).collect();
        for r in &self.config.rules {
            if let Rule::Binary { binary, label } = r {
                if !rows.iter().any(|x| x.binary == binary) {
                    rows.push(Row {
                        binary,
                        label: label.as_deref(),
                        checked: true,
                        streams: vec![],
                    });
                }
            }
        }
        rows.sort_by_key(|r| r.label.unwrap_or(r.binary).to_lowercase());

        let mut out: Vec<Element<'_, Message>> = vec![];
        for r in rows {
            let title = match r.label {
                Some(l) => format!("{l} ({})", r.binary),
                None => r.binary.to_string(),
            };
            let (status, ok) = group_status(&r.streams, r.checked);
            let binary = r.binary.to_string();
            let label = r.label.map(str::to_string);
            let st = text(status);
            let st = if ok { st.style(text::success) } else { st };
            out.push(
                row![
                    checkbox(r.checked).label(title).on_toggle(move |on| Message::ToggleBinary {
                        binary: binary.clone(),
                        label: label.clone(),
                        on,
                    }),
                    space().width(Fill),
                    st,
                ]
                .spacing(8)
                .into(),
            );
        }
        for v in self.snapshot.streams.iter().filter(|v| v.info.binary.is_none()) {
            let name = v
                .info
                .app_name
                .clone()
                .or_else(|| v.info.node_name.clone())
                .unwrap_or_else(|| format!("#{}", v.info.index));
            let (status, ok) = group_status(&[v], false);
            let st = text(status);
            let st = if ok { st.style(text::success) } else { st };
            out.push(
                row![
                    text(format!("{name} — нет application.process.binary, используйте regex")),
                    space().width(Fill),
                    st,
                ]
                .spacing(8)
                .into(),
            );
        }
        out
    }

    fn regex_row<'a>(&'a self, i: usize, field: &Field, pattern: &'a str) -> Element<'a, Message> {
        let status = match validate_pattern(pattern) {
            Err(e) => text(format!("ошибка: {}", e.lines().next().unwrap_or(""))).style(text::danger),
            Ok(()) => {
                let n = regex::Regex::new(pattern)
                    .map(|re| {
                        self.snapshot
                            .streams
                            .iter()
                            .filter(|v| v.info.field(field).is_some_and(|x| re.is_match(x)))
                            .count()
                    })
                    .unwrap_or(0);
                text(format!("совпадений: {n}"))
            }
        };
        row![
            pick_list(Field::ALL, Some(field.clone()), move |f| Message::RegexField(i, f)),
            text_input("шаблон, например (?i)factorio", pattern)
                .on_input(move |p| Message::RegexPattern(i, p))
                .width(Fill),
            status,
            button("✕").on_press(Message::RemoveRule(i)),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into()
    }
}

/// (status text, is_in_stream)
fn group_status(streams: &[&StreamView], checked: bool) -> (String, bool) {
    let has = |c| streams.iter().any(|v| v.capture == c);
    let (mut s, ok) = if has(CaptureStatus::InStream) {
        ("в стриме".to_string(), true)
    } else if has(CaptureStatus::Blocked) {
        ("заглушено".to_string(), false)
    } else if !streams.is_empty() {
        ("не захватывается".to_string(), false)
    } else {
        ("не играет".to_string(), false)
    };
    if !checked && streams.iter().any(|v| v.allowed) {
        s.push_str(" · разрешено regex");
    }
    (s, ok)
}
