# clickr

Turns an iPhone into a presentation remote over [iroh](https://iroh.computer).
The phone sends "next" and "previous" presses; the computer turns them into right
and left arrow key presses for whatever app has focus (Keynote, PowerPoint, Google
Slides in a browser, a PDF viewer, ...).

## Layout

```
crates/
  clickr-core/     shared library: protocol, Receiver (computer) and Remote (phone)
  clickr/          the `clickr` CLI, uses clickr-core and enigo for key presses
  clickr-ffi/      uniffi bindings over clickr-core, built as a static lib for iOS
  uniffi-bindgen/  runs uniffi-bindgen at the workspace's uniffi version
ios/
  project.yml      XcodeGen spec for Clickr.xcodeproj
  ClickrKit/       Swift package wrapping ClickrFFI.xcframework and the bindings
  Clickr/          SwiftUI app
scripts/
  build-ios.sh     builds the xcframework, generates bindings and the Xcode project
```

## Computer

```
cargo install --path crates/clickr
clickr accept
```

`clickr accept` prints a QR code and a ticket, then waits. The first time a phone
connects it asks whether to trust it; trusted phones are remembered. Pass `--yes`
to trust every phone that connects.

The receiver identity lives in the platform config directory (`~/Library/Application
Support/clickr` on macOS), so the ticket stays the same across restarts and a phone
only pairs once. Delete `secret.key` there to get a new identity, or edit
`trusted.txt` to revoke a phone.

On macOS, key presses need the Accessibility permission: System Settings, Privacy &
Security, Accessibility, then enable your terminal app.

## iPhone

Requires Xcode, [XcodeGen](https://github.com/yonaskolb/XcodeGen) and the iOS Rust
targets (the script installs the targets).

```
DEVELOPMENT_TEAM=<your team id> scripts/build-ios.sh
open ios/Clickr.xcodeproj
```

Run on your phone, scan the code printed by `clickr accept` (or paste the ticket),
and press Next and Back.

While Clickr is in front it disables auto-lock, so the screen stays on for the
whole talk. iOS does not let an app force itself to stay in the foreground; to lock
the phone into Clickr, turn on Guided Access (Settings, Accessibility, Guided
Access) and triple-click the side button while Clickr is open.

## Protocol

ALPN `n0/clickr/0`. The phone opens one bidirectional stream per connection and
writes one byte per press (`0x01` previous, `0x02` next). The computer answers each
byte with `0x00` (ok) or `0xFF` (unknown command). The phone reconnects on failure.

## Notes

`Cargo.lock` pins `libc` to 0.2.189 (and `mio` to 1.2.4 to allow that): libc
0.2.190+ restricts several Darwin types to macOS, which breaks `netdev` on iOS.
