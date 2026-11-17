//! Command line receiver for the clickr presentation remote.
//!
//! `clickr accept` prints a pairing ticket, waits for phones to connect, and turns their
//! button presses into left and right arrow key presses for whatever app has focus.

use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    sync::{Mutex, mpsc},
    time::Duration,
};

use clap::{Parser, Subcommand};
use clickr_core::{Command, CommandHandler, EndpointId, Receiver, SecretKey};
use n0_error::{Result, StdResultExt, anyerr};
use qrcode::{QrCode, render::unicode};
use tracing::warn;

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
        /// Trusts every phone that connects without asking.
        #[arg(long, short)]
        yes: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "clickr=info,clickr_core=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    match Cli::parse().command {
        Cmd::Accept { yes } => accept(yes).await,
    }
}

async fn accept(trust_all: bool) -> Result<()> {
    let dir = config_dir()?;
    let secret_key = load_or_create_key(&dir.join("secret.key"))?;
    let trusted = Trusted::load(dir.join("trusted.txt"))?;
    let keys = Keyboard::spawn()?;
    let handler = Handler {
        trusted,
        trust_all,
        keys,
        prompt: tokio::sync::Mutex::new(()),
    };

    let receiver = Receiver::spawn(secret_key, handler).await?;
    // Wait briefly for a relay connection so the ticket carries a relay URL. The ticket
    // works without it on the local network, so do not block forever when offline.
    if tokio::time::timeout(Duration::from_secs(5), receiver.endpoint().online())
        .await
        .is_err()
    {
        warn!("no relay connection yet, ticket only contains local addresses");
    }

    let ticket = receiver.ticket().to_string();
    let qr = QrCode::new(ticket.as_bytes()).std_context("failed to encode QR code")?;
    let image = qr
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    println!("{image}");
    println!("Scan the code with the Clickr app, or paste this ticket:\n\n{ticket}\n");
    println!("Waiting for remotes. Press Ctrl-C to stop.");

    tokio::signal::ctrl_c().await.anyerr()?;
    println!("\nShutting down.");
    receiver.shutdown().await;
    Ok(())
}

#[derive(Debug)]
struct Handler {
    trusted: Trusted,
    trust_all: bool,
    keys: Keyboard,
    /// Serializes interactive prompts so two phones never ask at the same time.
    prompt: tokio::sync::Mutex<()>,
}

impl CommandHandler for Handler {
    async fn authorize(&self, remote: EndpointId) -> bool {
        if self.trusted.contains(&remote) {
            return true;
        }
        let ok = if self.trust_all {
            true
        } else {
            let _guard = self.prompt.lock().await;
            ask_trust(remote).await
        };
        if ok && let Err(err) = self.trusted.insert(remote) {
            warn!("failed to save trusted remote: {err:#}");
        }
        ok
    }

    fn handle(&self, remote: EndpointId, command: Command) {
        println!("{} {:?}", remote.fmt_short(), command);
        self.keys.press(command);
    }
}

async fn ask_trust(remote: EndpointId) -> bool {
    tokio::task::spawn_blocking(move || {
        print!("Remote {} wants to connect. Trust it? [y/N] ", remote.fmt_short());
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

/// Sends key presses from a dedicated thread, since the platform input handle is not
/// `Send` on every OS.
#[derive(Debug)]
struct Keyboard {
    tx: mpsc::Sender<Command>,
}

impl Keyboard {
    fn spawn() -> Result<Self> {
        let (tx, rx) = mpsc::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        std::thread::Builder::new()
            .name("clickr-keys".into())
            .spawn(move || {
                use enigo::{Direction, Enigo, Key, Keyboard as _, Settings};
                let mut enigo = match Enigo::new(&Settings::default()) {
                    Ok(enigo) => {
                        ready_tx.send(Ok(())).ok();
                        enigo
                    }
                    Err(err) => {
                        let hint = if cfg!(target_os = "macos") {
                            "\n\nGrant your terminal app access in System Settings > \
                             Privacy & Security > Accessibility, then run clickr again."
                        } else {
                            ""
                        };
                        ready_tx
                            .send(Err(anyerr!("cannot simulate key presses: {err}{hint}")))
                            .ok();
                        return;
                    }
                };
                for command in rx {
                    let key = match command {
                        Command::Previous => Key::LeftArrow,
                        Command::Next => Key::RightArrow,
                    };
                    if let Err(err) = enigo.key(key, Direction::Click) {
                        warn!("failed to send key: {err}");
                    }
                }
            })
            .std_context("failed to spawn keyboard thread")?;
        ready_rx.recv().std_context("keyboard thread died")??;
        Ok(Self { tx })
    }

    fn press(&self, command: Command) {
        self.tx.send(command).ok();
    }
}

/// The set of remotes allowed to connect without a prompt, persisted one id per line.
#[derive(Debug)]
struct Trusted {
    path: PathBuf,
    ids: Mutex<BTreeSet<EndpointId>>,
}

impl Trusted {
    fn load(path: PathBuf) -> Result<Self> {
        let mut ids = BTreeSet::new();
        if path.exists() {
            let text = fs::read_to_string(&path).std_context("failed to read trusted remotes")?;
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                match line.parse() {
                    Ok(id) => {
                        ids.insert(id);
                    }
                    Err(err) => warn!("ignoring invalid trusted remote {line:?}: {err}"),
                }
            }
        }
        Ok(Self {
            path,
            ids: Mutex::new(ids),
        })
    }

    fn contains(&self, id: &EndpointId) -> bool {
        self.ids.lock().expect("poisoned").contains(id)
    }

    fn insert(&self, id: EndpointId) -> Result<()> {
        let mut ids = self.ids.lock().expect("poisoned");
        ids.insert(id);
        let text: String = ids.iter().map(|id| format!("{id}\n")).collect();
        fs::write(&self.path, text).std_context("failed to write trusted remotes")
    }
}

fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyerr!("no config directory on this platform"))?
        .join("clickr");
    fs::create_dir_all(&dir).std_context("failed to create config directory")?;
    Ok(dir)
}

/// Loads the receiver identity, creating it on first run.
///
/// A stable identity keeps tickets valid across restarts, so a phone only pairs once.
fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    if let Ok(bytes) = fs::read(path) {
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyerr!("corrupt secret key at {}", path.display()))?;
        return Ok(SecretKey::from_bytes(&bytes));
    }
    let key = SecretKey::generate();
    write_private(path, &key.to_bytes())?;
    Ok(key)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut file = opts
        .open(path)
        .std_context("failed to create secret key file")?;
    file.write_all(bytes)
        .std_context("failed to write secret key")
}
