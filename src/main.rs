mod config;
mod pulse;
mod rules;
mod tray;
mod ui;
mod wpcheck;

use std::fs::{File, OpenOptions, TryLockError};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::channel::mpsc;
use futures::{SinkExt, Stream, StreamExt};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};

use rules::Rule;

#[derive(Clone)]
pub struct TrayHandle(pub Arc<ksni::blocking::Handle<tray::AppTray>>);

impl std::fmt::Debug for TrayHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrayHandle")
    }
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Ready { core: pulse::CoreHandle, tray: Option<TrayHandle> },
    Snapshot(pulse::Snapshot),
    ShutdownComplete,
    CoreDied,
    ShowWindow,
    ToggleWindow,
    TrayOnline,
    TrayOffline,
    QuitRequested,
}

/// Rules loaded in `App::boot`, consumed by the worker when it starts the core.
pub static INITIAL_RULES: OnceLock<Vec<Rule>> = OnceLock::new();

pub fn worker() -> impl Stream<Item = AppEvent> {
    iced::stream::channel(100, async |mut output| {
        let (tx, mut rx) = mpsc::unbounded();
        let rules = INITIAL_RULES.get().cloned().unwrap_or_default();
        let core = pulse::spawn(rules, tx.clone());
        let tray = tray::spawn(tx.clone()).map(|h| TrayHandle(Arc::new(h)));
        listen_for_show(tx.clone());

        match signal_hook::iterator::Signals::new([SIGINT, SIGTERM, SIGHUP]) {
            Ok(mut signals) => {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    for _ in signals.forever() {
                        let _ = tx.unbounded_send(AppEvent::QuitRequested);
                    }
                });
            }
            Err(e) => eprintln!("cannot install signal handlers: {e}"),
        }

        let _ = output.send(AppEvent::Ready { core, tray }).await;
        while let Some(ev) = rx.next().await {
            let _ = output.send(ev).await;
        }
    })
}

fn runtime_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn socket_path() -> std::path::PathBuf {
    runtime_dir().join("discord-sound-capture.sock")
}

/// Ask the running instance to show its window. The primary may hold the lock
/// but not have bound the socket yet, so retry briefly.
fn notify_running() -> bool {
    use std::io::Write;
    for _ in 0..30 {
        if let Ok(mut s) = std::os::unix::net::UnixStream::connect(socket_path()) {
            return s.write_all(b"show\n").is_ok();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Listen for "show" requests from later launches. Only called while holding the lock,
/// so a leftover socket file is stale.
fn listen_for_show(tx: mpsc::UnboundedSender<AppEvent>) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot bind {}: {e}", path.display());
            return;
        }
    };
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            drop(conn);
            let _ = tx.unbounded_send(AppEvent::ShowWindow);
        }
    });
}

fn acquire_lock() -> Result<File, ()> {
    let dir = runtime_dir();
    let file = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join("discord-sound-capture.lock"))
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("cannot open lock file: {e}");
            return Err(());
        }
    };
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => {
            if notify_running() {
                std::process::exit(0);
            }
            eprintln!("discord-sound-capture is already running, but did not answer");
            Err(())
        }
        Err(TryLockError::Error(e)) => {
            eprintln!("lock failed: {e}");
            Err(())
        }
    }
}

fn main() -> std::process::ExitCode {
    let Ok(_lock) = acquire_lock() else {
        return std::process::ExitCode::FAILURE;
    };

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        if let Some(core) = pulse::CORE.get() {
            core.send(pulse::Command::Shutdown);
            std::thread::sleep(Duration::from_millis(300));
        }
    }));

    let res = iced::daemon(ui::App::boot, ui::App::update, ui::App::view)
        .title(|_: &ui::App, _| "Discord Sound Capture".to_string())
        .subscription(ui::App::subscription)
        .theme(iced::Theme::Dark)
        .run();
    match res {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}
