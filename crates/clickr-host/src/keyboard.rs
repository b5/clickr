//! Turns commands into arrow key presses.

use std::sync::mpsc;

use clickr_core::Command;
use n0_error::{Result, StdResultExt, anyerr};
use tracing::warn;

/// How many presses may queue up before new ones are dropped.
const QUEUE_LEN: usize = 32;

/// Sends key presses from a dedicated thread, since the platform input handle is not
/// `Send` on every OS.
#[derive(Debug)]
pub(crate) struct Keyboard {
    tx: mpsc::SyncSender<Command>,
}

impl Keyboard {
    /// Starts the keyboard thread.
    ///
    /// # Errors
    ///
    /// Returns an error if key presses cannot be simulated, for example because macOS
    /// has not granted the Accessibility permission.
    pub(crate) fn spawn() -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Command>(QUEUE_LEN);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<()>>(1);
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
                            "\n\nGrant Clickr (or your terminal app) access in System \
                             Settings > Privacy & Security > Accessibility, then start it \
                             again."
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

    /// Queues a key press, dropping it if the keyboard thread is backed up.
    pub(crate) fn press(&self, command: Command) {
        if self.tx.try_send(command).is_err() {
            warn!(?command, "dropped key press, keyboard thread is backed up");
        }
    }
}
