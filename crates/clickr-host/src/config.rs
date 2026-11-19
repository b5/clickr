//! Where the receiver keeps its files, and its identity.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use clickr_core::SecretKey;
use n0_error::{Result, StdResultExt, anyerr};

/// Returns the receiver's config directory, creating it if needed.
///
/// This is `~/Library/Application Support/clickr` on macOS.
///
/// # Errors
///
/// Returns an error if the platform has no config directory or it cannot be created.
pub fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyerr!("no config directory on this platform"))?
        .join("clickr");
    fs::create_dir_all(&dir).std_context("failed to create config directory")?;
    Ok(dir)
}

/// Loads the receiver identity from `dir`, creating it on first run.
///
/// A stable identity keeps tickets and issued tokens valid across restarts.
///
/// # Errors
///
/// Returns an error if the key file is corrupt or cannot be written.
pub(crate) fn load_or_create_key(dir: &Path) -> Result<SecretKey> {
    let path = dir.join("secret.key");
    if let Ok(bytes) = fs::read(&path) {
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyerr!("corrupt secret key at {}", path.display()))?;
        return Ok(SecretKey::from_bytes(&bytes));
    }
    let key = SecretKey::generate();
    write_private(&path, &key.to_bytes())?;
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

/// Returns a nickname for this computer: its macOS computer name, else its hostname.
pub fn default_nickname() -> String {
    let commands: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("scutil", &["--get", "ComputerName"]), ("hostname", &[])]
    } else {
        &[("hostname", &[])]
    };
    commands
        .iter()
        .find_map(|(program, args)| {
            let output = std::process::Command::new(program)
                .args(*args)
                .output()
                .ok()?;
            let name = String::from_utf8(output.stdout).ok()?.trim().to_string();
            (output.status.success() && !name.is_empty()).then_some(name)
        })
        .unwrap_or_else(|| "Computer".to_string())
}
