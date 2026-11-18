# clickr

Turns an iPhone into a presentation remote over [iroh](https://iroh.computer).
The phone sends "next" and "previous" presses; the computer turns them into right
and left arrow key presses for whatever app has focus (Keynote, PowerPoint, Google
Slides in a browser, a PDF viewer, ...).

## Layout

```
crates/
  clickr-core/     shared library: protocol, access tokens, Receiver (computer) and
                   Remote (phone)
  clickr-host/     receiver-side state shared by the app and CLI: identity, known
                   phones, revocations, key presses (enigo)
  clickr-app/      the Clickr Mac app (Dioxus desktop)
  clickr/          the `clickr` CLI
  clickr-ffi/      uniffi bindings over clickr-core, built as a static lib for iOS
  uniffi-bindgen/  runs uniffi-bindgen at the workspace's uniffi version
ios/
  project.yml      XcodeGen spec for Clickr.xcodeproj
  ClickrKit/       Swift package wrapping ClickrFFI.xcframework and the bindings
  Clickr/          SwiftUI app
scripts/
  build-ios.sh     builds the xcframework, generates bindings and the Xcode project
  build-mac.sh     bundles and ad-hoc signs the Mac app
```

## Computer

### Mac app

Requires the [Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started) 0.7.10
(`cargo install dioxus-cli@0.7.10`).

```
scripts/build-mac.sh
open target/dx/clickr-app/bundle/macos/macos/ClickrApp.app
```

The script bundles the app and ad-hoc signs it, so macOS remembers its Bluetooth and
Accessibility permissions. dx also writes a DMG next to the app, but it holds the
unsigned build. For development, run
`dx serve --platform desktop -p clickr-app` from the workspace root.

`crates/clickr-app/Info.plist` is the app's complete Info.plist: dx 0.7.10 ignores the
Bluetooth permission setting for macOS and replaces its own Info.plist with this file
instead of merging it. Keep its version in sync with `Cargo.toml`.

The window shows this computer's nickname (click Rename to change it), the pairing
code behind a Show/Hide toggle so it stays off a shared screen, the known phones with
a Revoke button, and revoked phones with Remove revocation. When a new phone asks to
connect, the window asks you to allow or deny it.

### CLI

```
cargo install --path crates/clickr
clickr accept
```

`clickr accept` prints a QR code and a ticket, then waits, asking on the terminal
before letting a new phone in. Pass `--yes` to allow every new phone. `clickr
devices` lists known and revoked phones, `clickr revoke <id>` and `clickr unrevoke
<id>` manage revocations (an id prefix is enough), and `clickr name <nickname>`
renames this computer.

### Both

The app and the CLI share the same identity and state in the platform config
directory (`~/Library/Application Support/clickr` on macOS): `secret.key` and
`state.json`. Run only one of them at a time, and stop it before using the CLI's
`revoke`, `unrevoke` or `name`, since a running receiver does not see those changes
and overwrites them. Delete `secret.key` to get a new identity; every phone then has
to pair again.

On macOS, key presses need the Accessibility permission: System Settings, Privacy &
Security, Accessibility, then enable Clickr (or your terminal app for the CLI).

## Pairing and access

- Each side has a nickname. The phone picks a random two-word name on first launch
  (rename it in the app's settings); the computer uses its computer name. Nicknames
  are exchanged on every connection.
- When a phone connects for the first time, the computer asks whether to allow it.
  Allowing it issues an [RCAN](https://github.com/n0-computer/rcan) capability token,
  `CONNECT: <computer endpoint id>`, signed by the computer and bound to the phone's
  endpoint id. The phone stores it and presents it on every later connection, so it
  is not asked about again. A copied token is useless on another device.
- Revoking a phone drops its connection and turns it away from then on, even with its
  token. Removing the revocation lets the same token work again.
- Both sides remember each other: the computer lists known phones, the phone lists
  known computers to pick from.
- Phones paired with an earlier version of clickr have to pair again once;
  `trusted.txt` is no longer read.

## Bluetooth

Besides the internet, the phone and computer reach each other over Bluetooth Low
Energy using [iroh-ble-transport](https://github.com/mcginty/iroh-ble-transport), so
clickr works when the two aren't on the same network or there is no network at all.
iroh picks the best path on its own; nothing needs to be configured.

- The first launch on each side asks for Bluetooth access. On macOS the permission
  goes to Clickr, or to your terminal app for the CLI (System Settings, Privacy &
  Security, Bluetooth).
- If Bluetooth is off or access was denied, clickr logs a warning and uses the
  internet only. Turning Bluetooth on later takes effect after restarting the
  receiver or the phone app.
- The phone and computer find each other over Bluetooth by endpoint id, so existing
  pairing tickets keep working.

## iPhone

Requires Xcode, [XcodeGen](https://github.com/yonaskolb/XcodeGen) and the iOS Rust
targets (the script installs the targets).

```
DEVELOPMENT_TEAM=<your team id> scripts/build-ios.sh
open ios/Clickr.xcodeproj
```

Run on your phone, tap Pair new computer, scan the code shown by the Mac app or
`clickr accept` (or paste the ticket), approve the phone on the computer, and press
Next and Back.

While Clickr is in front it disables auto-lock, so the screen stays on for the
whole talk. iOS does not let an app force itself to stay in the foreground; to lock
the phone into Clickr, turn on Guided Access (Settings, Accessibility, Guided
Access) and triple-click the side button while Clickr is open.

## Protocol

ALPN `n0/clickr/1`. The phone opens one bidirectional stream per connection and
starts with a handshake. Each handshake message is postcard-encoded with a u16
little-endian length prefix:

1. The phone sends `Hello { nickname, token }`, where `token` is its `CONNECT` token
   if it has one.
2. The computer answers `Granted { nickname, token }` (with `token` set when it just
   issued one), `Denied`, or `Revoked`.

After `Granted`, the phone writes one byte per press (`0x01` previous, `0x02` next),
and the computer answers each byte with `0x00` (ok) or `0xFF` (unknown command). The
phone reconnects on failure, presenting its token again.

## License

clickr is licensed under the [GNU Affero General Public License v3.0 or
later](LICENSE), matching its Bluetooth dependencies.

## Notes

`Cargo.lock` pins `libc` to 0.2.189 (and `mio` to 1.2.4 to allow that): libc
0.2.190+ restricts several Darwin types to macOS, which breaks `netdev` on iOS.
