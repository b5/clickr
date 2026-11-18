//! The Clickr Mac app: a receiver with a window.
//!
//! Shows the pairing code (hidden by default, since the screen is usually being
//! presented), asks before letting a new remote in, and lists known and revoked remotes
//! with the network path each connected remote uses.

use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use clickr_core::{Command, EndpointId, Paths};
use clickr_host::{Approver, Event, Host, HostState, KnownDevice, Revocation, ago, config_dir};
use dioxus::{
    desktop::{Config, LogicalSize, WindowBuilder},
    prelude::*,
};
use qrcode::{QrCode, render::svg};
use tokio::sync::{broadcast::error::RecvError, mpsc, oneshot};

/// How many approval requests may wait for the user at once.
const PENDING_REQUESTS: usize = 8;

const STYLE: &str = include_str!("style.css");

fn main() {
    let window = WindowBuilder::new()
        .with_title("Clickr")
        .with_inner_size(LogicalSize::new(440.0, 720.0))
        .with_min_inner_size(LogicalSize::new(380.0, 480.0));
    dioxus::LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window).with_menu(None))
        .launch(App);
}

/// A remote waiting for the user to allow or deny it.
#[derive(Debug)]
struct Request {
    id: EndpointId,
    nickname: String,
    reply: oneshot::Sender<bool>,
}

/// Forwards approval questions to the window.
#[derive(Debug)]
struct WindowApprover {
    requests: mpsc::Sender<Request>,
}

impl Approver for WindowApprover {
    async fn approve(&self, id: EndpointId, nickname: &str) -> bool {
        let (reply, answer) = oneshot::channel();
        let request = Request {
            id,
            nickname: nickname.to_string(),
            reply,
        };
        if self.requests.send(request).await.is_err() {
            return false;
        }
        answer.await.unwrap_or(false)
    }
}

/// What the window shows, mirrored from the [`Host`].
#[derive(Clone, Copy, PartialEq)]
struct Live {
    state: Signal<HostState>,
    connected: Signal<BTreeSet<EndpointId>>,
    paths: Signal<BTreeMap<EndpointId, Paths>>,
    requests: Signal<Vec<Request>>,
    last_press: Signal<Option<(EndpointId, Command)>>,
}

/// Starts the host and keeps the signals in sync with it until the window closes.
async fn run_host(live: Live) -> Result<Rc<Host>, String> {
    let Live {
        mut state,
        mut connected,
        mut paths,
        mut requests,
        mut last_press,
    } = live;
    let (tx, mut rx) = mpsc::channel(PENDING_REQUESTS);
    let dir = config_dir().map_err(|err| format!("{err:#}"))?;
    let host = Host::start(&dir, WindowApprover { requests: tx })
        .await
        .map_err(|err| format!("{err:#}"))?;
    let host = Rc::new(host);
    state.set(host.snapshot());

    spawn(async move {
        while let Some(request) = rx.recv().await {
            requests.write().push(request);
        }
    });

    let mut events = host.subscribe();
    let watched = host.clone();
    spawn(async move {
        loop {
            match events.recv().await {
                Ok(Event::StateChanged) => state.set(watched.snapshot()),
                Ok(Event::Connected { .. } | Event::Disconnected { .. }) => {
                    connected.set(watched.connected());
                    paths.set(watched.paths());
                }
                Ok(Event::PathsChanged { id, paths: changed }) => {
                    paths.write().insert(id, changed);
                }
                Ok(Event::Pressed { id, command }) => last_press.set(Some((id, command))),
                Err(RecvError::Lagged(_)) => {
                    state.set(watched.snapshot());
                    connected.set(watched.connected());
                    paths.set(watched.paths());
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
    Ok(host)
}

#[component]
fn App() -> Element {
    let live = Live {
        state: use_signal(HostState::default),
        connected: use_signal(BTreeSet::new),
        paths: use_signal(BTreeMap::new),
        requests: use_signal(Vec::new),
        last_press: use_signal(|| None),
    };
    let mut host = use_resource(move || run_host(live));

    let body = match &*host.read() {
        None => rsx! { p { class: "muted center", "Starting..." } },
        Some(Err(err)) => rsx! {
            div { class: "card error",
                h2 { "Clickr could not start" }
                pre { "{err}" }
                button { onclick: move |_| host.restart(), "Try again" }
            }
        },
        Some(Ok(host)) => rsx! {
            Main { host: HostHandle(host.clone()), live }
        },
    };
    rsx! {
        style { {STYLE} }
        main { {body} }
    }
}

/// A shared [`Host`] that can be passed as a component prop.
#[derive(Clone)]
struct HostHandle(Rc<Host>);

impl PartialEq for HostHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

#[component]
fn Main(host: HostHandle, live: Live) -> Element {
    let snapshot = live.state.read().clone();
    let online = live.connected.read().clone();
    let paths = live.paths.read().clone();
    let press = (live.last_press)().map(|(id, command)| {
        let name = snapshot
            .known
            .get(&id)
            .map_or_else(|| id.fmt_short().to_string(), |d| d.nickname.clone());
        let label = match command {
            Command::Next => "Next",
            Command::Previous => "Back",
        };
        format!("{name}: {label}")
    });

    rsx! {
        Nickname { host: host.clone(), nickname: snapshot.nickname.clone() }
        PairingCode { host: host.clone() }
        section { class: "card",
            h2 { "Remotes" }
            if snapshot.known.is_empty() {
                p { class: "muted", "No remotes yet. Show the pairing code and scan it with the Clickr app." }
            }
            for (id, device) in snapshot.known.clone() {
                DeviceRow {
                    key: "{id}",
                    host: host.clone(),
                    id,
                    device,
                    online: online.contains(&id),
                    paths: paths.get(&id).cloned(),
                }
            }
            if let Some(press) = press {
                p { class: "muted last-press", "Last press: {press}" }
            }
        }
        if !snapshot.revoked.is_empty() {
            section { class: "card",
                h2 { "Revoked" }
                p { class: "muted", "These remotes are turned away until you remove the revocation." }
                for (id, revocation) in snapshot.revoked.clone() {
                    RevokedRow { key: "{id}", host: host.clone(), id, revocation }
                }
            }
        }
        ApprovalPrompt { requests: live.requests }
    }
}

/// The host nickname, editable in place.
#[component]
fn Nickname(host: HostHandle, nickname: String) -> Element {
    let mut editing = use_signal(|| false);
    let mut draft = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);

    let save = {
        let host = host.clone();
        move || {
            let name = draft.read().trim().to_string();
            if name.is_empty() {
                return;
            }
            match host.0.set_nickname(name) {
                Ok(()) => {
                    editing.set(false);
                    error.set(None);
                }
                Err(err) => error.set(Some(format!("{err:#}"))),
            }
        }
    };

    rsx! {
        header { class: "title",
            if editing() {
                input {
                    class: "nickname-input",
                    value: "{draft}",
                    autofocus: true,
                    oninput: move |event| draft.set(event.value()),
                    onkeydown: {
                        let mut save = save.clone();
                        move |event: KeyboardEvent| {
                        if event.key() == Key::Enter {
                            save();
                        } else if event.key() == Key::Escape {
                            editing.set(false);
                        }
                    }},
                }
                button {
                    onclick: {
                        let mut save = save.clone();
                        move |_| save()
                    },
                    "Save"
                }
            } else {
                div {
                    h1 { "{nickname}" }
                    p { class: "muted", "Remotes see this computer by this name." }
                }
                button {
                    class: "secondary",
                    onclick: move |_| {
                        draft.set(nickname.clone());
                        editing.set(true);
                    },
                    "Rename"
                }
            }
        }
        if let Some(err) = error() {
            p { class: "error-text", "{err}" }
        }
    }
}

/// The pairing QR code and ticket, hidden until asked for.
#[component]
fn PairingCode(host: HostHandle) -> Element {
    let mut shown = use_signal(|| false);
    let mut copied = use_signal(|| false);

    let content = if shown() {
        let ticket = host.0.ticket().to_string();
        let qr = QrCode::new(ticket.as_bytes()).map(|code| {
            code.render::<svg::Color>()
                .min_dimensions(240, 240)
                .quiet_zone(true)
                .build()
        });
        let copy_script = format!(
            "navigator.clipboard.writeText({})",
            serde_json::to_string(&ticket).unwrap_or_default()
        );
        rsx! {
            match qr {
                Ok(svg) => rsx! { div { class: "qr", dangerous_inner_html: svg } },
                Err(err) => rsx! { p { class: "error-text", "Could not draw the code: {err}" } },
            }
            p { class: "muted", "Scan with the Clickr app on your iPhone, or paste the ticket." }
            code { class: "ticket", "{ticket}" }
            button {
                class: "secondary",
                onclick: move |_| {
                    document::eval(&copy_script);
                    copied.set(true);
                },
                if copied() { "Copied" } else { "Copy ticket" }
            }
        }
    } else {
        rsx! {
            p { class: "muted", "The code is hidden so it doesn't end up on a shared screen." }
        }
    };

    rsx! {
        section { class: "card",
            div { class: "row",
                h2 { "Pairing code" }
                button {
                    onclick: move |_| {
                        shown.toggle();
                        copied.set(false);
                    },
                    if shown() { "Hide" } else { "Show" }
                }
            }
            {content}
        }
    }
}

#[component]
fn DeviceRow(
    host: HostHandle,
    id: EndpointId,
    device: KnownDevice,
    online: bool,
    paths: Option<Paths>,
) -> Element {
    let mut confirming = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let status = match (&paths, online) {
        (Some(paths), true) => format!("Connected {paths}"),
        (None, true) => "Connected".to_string(),
        (_, false) => format!("Last seen {}", ago(device.last_seen)),
    };
    let badge = paths
        .as_ref()
        .filter(|_| online)
        .and_then(|paths| paths.selected)
        .map(|transport| transport.to_string());

    rsx! {
        div { class: "device",
            span { class: if online { "dot online" } else { "dot" } }
            div { class: "device-text",
                div { class: "device-name",
                    span { "{device.nickname}" }
                    if let Some(badge) = badge {
                        span { class: "badge", "{badge}" }
                    }
                }
                div { class: "muted small", span { class: "mono", "{id.fmt_short()}" } " - {status}" }
                if let Some(err) = error() {
                    div { class: "error-text small", "{err}" }
                }
            }
            if confirming() {
                button { class: "secondary", onclick: move |_| confirming.set(false), "Cancel" }
                button {
                    class: "danger",
                    onclick: move |_| {
                        confirming.set(false);
                        if let Err(err) = host.0.revoke(id) {
                            error.set(Some(format!("{err:#}")));
                        }
                    },
                    "Revoke"
                }
            } else {
                button { class: "secondary", onclick: move |_| confirming.set(true), "Revoke..." }
            }
        }
    }
}

#[component]
fn RevokedRow(host: HostHandle, id: EndpointId, revocation: Revocation) -> Element {
    let mut error = use_signal(|| None::<String>);
    let name = if revocation.nickname.is_empty() {
        "Unknown remote".to_string()
    } else {
        revocation.nickname.clone()
    };

    rsx! {
        div { class: "device",
            span { class: "dot revoked" }
            div { class: "device-text",
                div { "{name}" }
                div { class: "muted small",
                    span { class: "mono", "{id.fmt_short()}" }
                    " - revoked {ago(revocation.revoked_at)}"
                }
                if let Some(err) = error() {
                    div { class: "error-text small", "{err}" }
                }
            }
            button {
                class: "secondary",
                onclick: move |_| {
                    if let Err(err) = host.0.unrevoke(&id) {
                        error.set(Some(format!("{err:#}")));
                    }
                },
                "Remove revocation"
            }
        }
    }
}

/// Asks about the oldest waiting remote, one at a time.
#[component]
fn ApprovalPrompt(requests: Signal<Vec<Request>>) -> Element {
    // A remote that gave up waiting closes its side of the reply channel. Skip those
    // here and prune them when answering: writing the signal while rendering it would
    // re-render forever.
    let current = requests
        .read()
        .iter()
        .find(|request| !request.reply.is_closed())
        .map(|request| (request.id, request.nickname.clone()));
    let Some((id, nickname)) = current else {
        return rsx! {};
    };

    let mut answer = move |allow: bool| {
        let mut queue = requests.write();
        queue.retain(|request| !request.reply.is_closed());
        if let Some(index) = queue.iter().position(|request| request.id == id) {
            queue.remove(index).reply.send(allow).ok();
        }
    };

    rsx! {
        div { class: "overlay",
            div { class: "dialog",
                h2 { "{nickname} wants to connect" }
                p { class: "muted",
                    "Remote "
                    span { class: "mono", "{id.fmt_short()}" }
                    ". Allowing it gives it a token, so it can connect without asking again until you revoke it."
                }
                div { class: "actions",
                    button { class: "secondary", onclick: move |_| answer(false), "Deny" }
                    button { onclick: move |_| answer(true), "Allow" }
                }
            }
        }
    }
}
