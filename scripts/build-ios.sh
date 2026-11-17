#!/usr/bin/env bash
# Builds clickr-ffi for iOS, generates the Swift bindings, packages everything as
# ios/ClickrKit/ClickrFFI.xcframework, and regenerates the Xcode project.
#
# Usage: scripts/build-ios.sh [--debug]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PROFILE=release
CARGO_FLAGS=(--release)
if [[ "${1:-}" == "--debug" ]]; then
  PROFILE=debug
  CARGO_FLAGS=()
fi

TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)
KIT="$ROOT/ios/ClickrKit"
BUILD="$ROOT/ios/build"
HEADERS="$BUILD/headers"
XCFRAMEWORK="$KIT/ClickrFFI.xcframework"

rustup target add "${TARGETS[@]}" >/dev/null

export IPHONEOS_DEPLOYMENT_TARGET=17.0
for target in "${TARGETS[@]}"; do
  echo "==> cargo build $target ($PROFILE)"
  cargo build -p clickr-ffi --lib --target "$target" ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}
done

# uniffi reads the interface from the host cdylib; the interface is identical across
# targets.
echo "==> generating Swift bindings"
cargo build -p clickr-ffi --lib ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}
rm -rf "$BUILD/bindings" "$HEADERS"
mkdir -p "$BUILD/bindings" "$HEADERS" "$KIT/Sources/ClickrKit"
cargo run -q -p uniffi-bindgen -- generate \
  --library "target/$PROFILE/libclickr_ffi.dylib" \
  --language swift \
  --out-dir "$BUILD/bindings"

cp "$BUILD/bindings/clickr_ffi.swift" "$KIT/Sources/ClickrKit/clickr_ffi.swift"
cp "$BUILD/bindings/clickr_ffiFFI.h" "$HEADERS/"
# Xcode only picks up a module map inside an xcframework if it is named module.modulemap.
cp "$BUILD/bindings/clickr_ffiFFI.modulemap" "$HEADERS/module.modulemap"

echo "==> creating $XCFRAMEWORK"
rm -rf "$XCFRAMEWORK"
args=()
for target in "${TARGETS[@]}"; do
  args+=(-library "target/$target/$PROFILE/libclickr_ffi.a" -headers "$HEADERS")
done
xcodebuild -create-xcframework "${args[@]}" -output "$XCFRAMEWORK" >/dev/null

echo "==> generating Xcode project"
(cd ios && xcodegen generate)

echo "Done. Open ios/Clickr.xcodeproj"
