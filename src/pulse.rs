use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedSender;
use libpulse_binding::callbacks::ListResult;
use libpulse_binding::context::introspect::{Introspector, SinkInputInfo, SourceOutputInfo};
use libpulse_binding::context::subscribe::{Facility, InterestMaskSet, Operation as SubOp};
use libpulse_binding::context::{Context, FlagSet, State as CtxState};
use libpulse_binding::mainloop::standard::Mainloop;
use libpulse_binding::time::MicroSeconds;

use crate::AppEvent;
use crate::rules::{Rule, RuleSet, StreamInfo, desired_mute};

const CAPTURE_APP: &str = "discord_capture";

pub enum Command {
    SetRules(Vec<Rule>),
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStatus {
    NotCaptured,
    InStream,
    Blocked,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamView {
    pub info: StreamInfo,
    pub capture: CaptureStatus,
    pub allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub connected: bool,
    pub streams: Vec<StreamView>,
}

#[derive(Clone)]
pub struct CoreHandle {
    tx: Sender<Command>,
}

impl CoreHandle {
    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }
}

impl std::fmt::Debug for CoreHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CoreHandle")
    }
}

pub static CORE: OnceLock<CoreHandle> = OnceLock::new();

pub fn spawn(initial_rules: Vec<Rule>, events: UnboundedSender<AppEvent>) -> CoreHandle {
    let (tx, rx) = mpsc::channel();
    let handle = CoreHandle { tx };
    let _ = CORE.set(handle.clone());
    let ev = events.clone();
    std::thread::Builder::new()
        .name("pulse-core".into())
        .spawn(move || {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(rx, ev.clone(), initial_rules)
            }));
            if !matches!(res, Ok(true)) {
                let _ = ev.unbounded_send(AppEvent::CoreDied);
            }
        })
        .expect("spawn pulse-core");
    handle
}

struct Capture {
    target: Option<u32>,
    muted: bool,
}

struct State {
    events: UnboundedSender<AppEvent>,
    introspect: Option<Rc<RefCell<Introspector>>>,
    connected: bool,
    streams: HashMap<u32, StreamInfo>,
    captures: HashMap<u32, Capture>,
    rules: RuleSet,
    shutting_down: bool,
    pending_ops: u32,
    reconcile_in_flight: bool,
    last_sent: Option<Snapshot>,
}

type S = Rc<RefCell<State>>;

fn sink_info(i: &SinkInputInfo) -> StreamInfo {
    let g = |k: &str| i.proplist.get_str(k);
    StreamInfo {
        index: i.index,
        app_name: g("application.name"),
        binary: g("application.process.binary"),
        media_name: g("media.name"),
        node_name: g("node.name"),
    }
}

/// Some(capture) iff the source output is a discord capture.
fn capture_info(i: &SourceOutputInfo) -> Option<Capture> {
    if i.proplist.get_str("application.name").as_deref() != Some(CAPTURE_APP) {
        return None;
    }
    Some(Capture {
        target: i.proplist.get_str("target.object").and_then(|s| s.parse().ok()),
        muted: i.mute,
    })
}

fn emit(s: &S) {
    let mut st = s.borrow_mut();
    let snap = if st.connected {
        let mut streams: Vec<StreamView> = st
            .streams
            .values()
            .map(|info| {
                let capture = match st.captures.values().find(|c| c.target == Some(info.index)) {
                    Some(c) if c.muted => CaptureStatus::Blocked,
                    Some(_) => CaptureStatus::InStream,
                    None => CaptureStatus::NotCaptured,
                };
                StreamView { info: info.clone(), capture, allowed: st.rules.allows(info) }
            })
            .collect();
        streams.sort_by_key(|v| {
            let n = v.info.app_name.as_deref().or(v.info.binary.as_deref()).unwrap_or("?");
            (n.to_lowercase(), v.info.index)
        });
        Snapshot { connected: true, streams }
    } else {
        Snapshot { connected: false, streams: vec![] }
    };
    if st.last_sent.as_ref() != Some(&snap) {
        st.last_sent = Some(snap.clone());
        let _ = st.events.unbounded_send(AppEvent::Snapshot(snap));
    }
}

fn enforce(s: &S, idx: u32) {
    let mut guard = s.borrow_mut();
    let st = &mut *guard;
    let Some(cap) = st.captures.get_mut(&idx) else { return };
    let want = desired_mute(cap.target, &st.streams, &st.rules, st.shutting_down);
    if cap.muted == want {
        return;
    }
    cap.muted = want;
    let Some(intro) = st.introspect.clone() else { return };
    st.pending_ops += 1;
    let s2 = s.clone();
    let _ = intro.borrow_mut().set_source_output_mute(
        idx,
        want,
        Some(Box::new(move |ok| {
            let mut st = s2.borrow_mut();
            st.pending_ops = st.pending_ops.saturating_sub(1);
            if !ok {
                eprintln!("mute op failed for source-output {idx}");
            }
        })),
    );
}

fn enforce_all(s: &S) {
    let idxs: Vec<u32> = s.borrow().captures.keys().copied().collect();
    for i in idxs {
        enforce(s, i);
    }
}

fn on_event(s: &S, fac: Option<Facility>, op: Option<SubOp>, idx: u32) {
    let (Some(fac), Some(op)) = (fac, op) else { return };
    match (fac, op) {
        (Facility::SinkInput, SubOp::Removed) => {
            s.borrow_mut().streams.remove(&idx);
            emit(s);
        }
        (Facility::SinkInput, _) => {
            let intro = s.borrow().introspect.clone();
            let Some(intro) = intro else { return };
            let s2 = s.clone();
            let _ = intro.borrow().get_sink_input_info(idx, move |r| {
                if let ListResult::Item(i) = r {
                    s2.borrow_mut().streams.insert(idx, sink_info(i));
                    let targets: Vec<u32> = s2
                        .borrow()
                        .captures
                        .iter()
                        .filter(|(_, c)| c.target == Some(idx))
                        .map(|(k, _)| *k)
                        .collect();
                    for t in targets {
                        enforce(&s2, t);
                    }
                    emit(&s2);
                }
            });
        }
        (Facility::SourceOutput, SubOp::Removed) => {
            s.borrow_mut().captures.remove(&idx);
            emit(s);
        }
        (Facility::SourceOutput, op) => {
            let intro = s.borrow().introspect.clone();
            let Some(intro) = intro else { return };
            let s2 = s.clone();
            let _ = intro.borrow().get_source_output_info(idx, move |r| {
                if let ListResult::Item(i) = r {
                    match capture_info(i) {
                        Some(c) => {
                            s2.borrow_mut().captures.insert(idx, c);
                            enforce(&s2, idx);
                            emit(&s2);
                        }
                        None => {
                            let removed =
                                op == SubOp::Changed && s2.borrow_mut().captures.remove(&idx).is_some();
                            if removed {
                                emit(&s2);
                            }
                        }
                    }
                }
            });
        }
        _ => {}
    }
}

fn reconcile(s: &S) {
    let intro = {
        let mut st = s.borrow_mut();
        if st.reconcile_in_flight {
            return;
        }
        let Some(i) = st.introspect.clone() else { return };
        st.reconcile_in_flight = true;
        i
    };
    let s1 = s.clone();
    let mut new_streams: HashMap<u32, StreamInfo> = HashMap::new();
    let _ = intro.borrow().get_sink_input_info_list(move |r| match r {
        ListResult::Item(i) => {
            new_streams.insert(i.index, sink_info(i));
        }
        ListResult::Error => s1.borrow_mut().reconcile_in_flight = false,
        ListResult::End => {
            s1.borrow_mut().streams = std::mem::take(&mut new_streams);
            let intro = s1.borrow().introspect.clone();
            let Some(intro) = intro else {
                s1.borrow_mut().reconcile_in_flight = false;
                return;
            };
            let s2 = s1.clone();
            let mut new_caps: HashMap<u32, Capture> = HashMap::new();
            let _ = intro.borrow().get_source_output_info_list(move |r| match r {
                ListResult::Item(i) => {
                    if let Some(c) = capture_info(i) {
                        new_caps.insert(i.index, c);
                    }
                }
                ListResult::Error => s2.borrow_mut().reconcile_in_flight = false,
                ListResult::End => {
                    {
                        let mut st = s2.borrow_mut();
                        st.captures = std::mem::take(&mut new_caps);
                        st.reconcile_in_flight = false;
                    }
                    enforce_all(&s2);
                    emit(&s2);
                }
            });
        }
    });
}

fn run(cmd_rx: Receiver<Command>, events: UnboundedSender<AppEvent>, initial: Vec<Rule>) -> bool {
    let s: S = Rc::new(RefCell::new(State {
        events: events.clone(),
        introspect: None,
        connected: false,
        streams: HashMap::new(),
        captures: HashMap::new(),
        rules: RuleSet::new(initial),
        shutting_down: false,
        pending_ops: 0,
        reconcile_in_flight: false,
        last_sent: None,
    }));
    let Some(mut mainloop) = Mainloop::new() else {
        eprintln!("cannot create pulse mainloop");
        return false;
    };
    let mut ctx: Option<Context> = None;
    let mut retry_at = Instant::now();
    let mut next_reconcile = Instant::now();
    let mut deadline = Instant::now();
    emit(&s);

    loop {
        loop {
            match cmd_rx.try_recv() {
                Ok(Command::SetRules(r)) => {
                    s.borrow_mut().rules = RuleSet::new(r);
                    enforce_all(&s);
                    emit(&s);
                }
                Ok(Command::Shutdown) | Err(TryRecvError::Disconnected) => {
                    if !s.borrow().shutting_down {
                        s.borrow_mut().shutting_down = true;
                        enforce_all(&s);
                        deadline = Instant::now() + Duration::from_secs(1);
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        {
            let st = s.borrow();
            if st.shutting_down && (!st.connected || st.pending_ops == 0 || Instant::now() >= deadline) {
                drop(st);
                s.borrow_mut().introspect = None;
                if let Some(c) = ctx.as_mut() {
                    c.disconnect();
                }
                drop(ctx);
                drop(mainloop);
                let _ = events.unbounded_send(AppEvent::ShutdownComplete);
                return true;
            }
        }

        if ctx.is_none() {
            if Instant::now() < retry_at {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            match Context::new(&mainloop, "discord-sound-capture") {
                Some(mut c) => {
                    if c.connect(None, FlagSet::NOFLAGS, None).is_ok() {
                        ctx = Some(c);
                    } else {
                        retry_at = Instant::now() + Duration::from_secs(1);
                        continue;
                    }
                }
                None => {
                    retry_at = Instant::now() + Duration::from_secs(1);
                    continue;
                }
            }
        }

        let mut failed = false;
        if let Some(c) = ctx.as_mut() {
            match c.get_state() {
                CtxState::Ready if !s.borrow().connected => {
                    let s2 = s.clone();
                    c.set_subscribe_callback(Some(Box::new(move |f, o, i| on_event(&s2, f, o, i))));
                    let _ = c.subscribe(
                        InterestMaskSet::SINK_INPUT | InterestMaskSet::SOURCE_OUTPUT,
                        |_| {},
                    );
                    {
                        let mut st = s.borrow_mut();
                        st.connected = true;
                        st.introspect = Some(Rc::new(RefCell::new(c.introspect())));
                    }
                    reconcile(&s);
                    next_reconcile = Instant::now() + Duration::from_secs(1);
                }
                CtxState::Failed | CtxState::Terminated => failed = true,
                _ => {}
            }
        }
        if !failed && s.borrow().connected && Instant::now() >= next_reconcile {
            reconcile(&s);
            next_reconcile = Instant::now() + Duration::from_secs(1);
        }

        if !failed {
            failed = !(mainloop.prepare(Some(MicroSeconds(50_000))).is_ok()
                && mainloop.poll().is_ok()
                && mainloop.dispatch().is_ok());
        }

        if failed {
            s.borrow_mut().introspect = None;
            if let Some(c) = ctx.as_mut() {
                c.disconnect();
            }
            ctx = None;
            {
                let mut st = s.borrow_mut();
                st.connected = false;
                st.streams.clear();
                st.captures.clear();
                st.pending_ops = 0;
                st.reconcile_in_flight = false;
            }
            emit(&s);
            retry_at = Instant::now() + Duration::from_secs(1);
        }
    }
}
