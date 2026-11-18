//! Command line receiver for the clickr presentation remote.
//!
//! `clickr accept` prints a pairing ticket, waits for phones to connect, and turns their
//! button presses into left and right arrow key presses for whatever app has focus. The
//! other subcommands manage known phones and revocations.

use std::io::{BufRead, Write};

use clap::{Parser, Subcommand};
use clickr_core::EndpointId;
use clickr_host::{Approver, Event, Host, HostState, Store, ago, config_dir, default_nickname};
use n0_error::{Result, StdResultExt, anyerr};
use qrcode::{QrCode, render::unicode};
use tokio::sync::broadcast::error::RecvError;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Prints a pairing ticket and turns remote presses into arrow key presses.
    Accept {
        /// Allows every new phone without asking.
        #[arg(long, short)]
        yes: bool,
    },
    /// Lists known and revoked phones.
    Devices,
    /// Revokes a phone, turning it away until unrevoked. Stop `accept` first.
    Revoke {
        /// The phone's endpoint id, or a unique prefix of it.
        id: String,
    },
    /// Removes a revocation, so the phone's existing token works again. Stop `accept`
    /// first.
    Unrevoke {
        /// The phone's endpoint id, or a unique prefix of it.
        id: String,
    },
    /// Shows or sets the nickname phones see for this computer. Stop `accept` first.
    Name {
        /// The new nickname. Omit to print the current one.
        nickname: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "clickr=info,clickr_core=info,clickr_host=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    match Cli::parse().command {
        Cmd::Accept { yes } => accept(yes).await,
        Cmd::Devices => {
            print_devices(&open_store()?.snapshot());
            Ok(())
        }
        Cmd::Revoke { id } => {
            let store = open_store()?;
            let id = find_id(&store.snapshot(), &id)?;
            store.revoke(id)?;
            println!("Revoked {}.", id.fmt_short());
            Ok(())
        }
        Cmd::Unrevoke { id } => {
            let store = open_store()?;
            let id = find_id(&store.snapshot(), &id)?;
            store.unrevoke(&id)?;
            println!("Removed the revocation of {}.", id.fmt_short());
            Ok(())
        }
        Cmd::Name { nickname: None } => {
            println!("{}", open_store()?.nickname());
            Ok(())
        }
        Cmd::Name {
            nickname: Some(nickname),
        } => open_store()?.set_nickname(nickname),
    }
}

async fn accept(allow_all: bool) -> Result<()> {
    let host = Host::start(&config_dir()?, TerminalApprover::new(allow_all)).await?;

    let ticket = host.ticket().to_string();
    let qr = QrCode::new(ticket.as_bytes()).std_context("failed to encode QR code")?;
    let image = qr
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    println!("{image}");
    println!(
        "{} is ready. Scan the code with the Clickr app, or paste this ticket:\n\n{ticket}\n",
        host.snapshot().nickname
    );
    println!("Waiting for remotes. Press Ctrl-C to stop.");

    let mut events = host.subscribe();
    loop {
        tokio::select! {
            res = tokio::signal::ctrl_c() => {
                res.anyerr()?;
                break;
            }
            event = events.recv() => match event {
                Ok(event) => print_event(&host, &event),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
        }
    }
    println!("\nShutting down.");
    host.shutdown().await;
    Ok(())
}

fn print_event(host: &Host, event: &Event) {
    match event {
        Event::Connected { id, nickname } => {
            println!("{nickname} ({}) connected", id.fmt_short());
        }
        Event::Disconnected { id } => {
            println!("{} disconnected", display_name(&host.snapshot(), id));
        }
        Event::Pressed { id, command } => {
            println!("{} {command:?}", display_name(&host.snapshot(), id));
        }
        Event::StateChanged => {}
    }
}

/// Asks on the terminal, or allows everyone with `--yes`.
#[derive(Debug)]
struct TerminalApprover {
    allow_all: bool,
    /// Serializes prompts so two phones never ask at the same time.
    prompt: tokio::sync::Mutex<()>,
}

impl TerminalApprover {
    fn new(allow_all: bool) -> Self {
        Self {
            allow_all,
            prompt: tokio::sync::Mutex::new(()),
        }
    }
}

impl Approver for TerminalApprover {
    async fn approve(&self, id: EndpointId, nickname: &str) -> bool {
        if self.allow_all {
            println!("Allowing {nickname} ({}).", id.fmt_short());
            return true;
        }
        let _guard = self.prompt.lock().await;
        let question = format!(
            "{nickname} ({}) wants to connect. Allow? [y/N] ",
            id.fmt_short()
        );
        tokio::task::spawn_blocking(move || {
            print!("{question}");
            std::io::stdout().flush().ok();
            let mut line = String::new();
            if std::io::stdin().lock().read_line(&mut line).is_err() {
                return false;
            }
            matches!(line.trim(), "y" | "Y" | "yes")
        })
        .await
        .unwrap_or(false)
    }
}

fn open_store() -> Result<Store> {
    Store::open(&config_dir()?, default_nickname)
}

fn print_devices(state: &HostState) {
    if state.known.is_empty() && state.revoked.is_empty() {
        println!("No phones yet. Run `clickr accept` to pair one.");
        return;
    }
    if !state.known.is_empty() {
        println!("Known phones:");
        for (id, device) in &state.known {
            println!(
                "  {}  {}  last seen {}",
                id.fmt_short(),
                device.nickname,
                ago(device.last_seen)
            );
        }
    }
    if !state.revoked.is_empty() {
        println!("Revoked phones:");
        for (id, revocation) in &state.revoked {
            println!(
                "  {}  {}  revoked {}",
                id.fmt_short(),
                revocation.nickname,
                ago(revocation.revoked_at)
            );
        }
    }
}

/// Resolves a full endpoint id or a unique prefix among known and revoked phones.
fn find_id(state: &HostState, query: &str) -> Result<EndpointId> {
    if let Ok(id) = query.parse() {
        return Ok(id);
    }
    let matches: Vec<EndpointId> = state
        .known
        .keys()
        .chain(state.revoked.keys())
        .filter(|id| id.to_string().starts_with(query))
        .copied()
        .collect();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(anyerr!("no known or revoked phone matches {query:?}")),
        _ => Err(anyerr!(
            "{query:?} matches several phones, use more characters"
        )),
    }
}

fn display_name(state: &HostState, id: &EndpointId) -> String {
    match state.known.get(id) {
        Some(device) => format!("{} ({})", device.nickname, id.fmt_short()),
        None => id.fmt_short().to_string(),
    }
}
