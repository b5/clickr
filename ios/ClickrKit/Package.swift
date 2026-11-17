// swift-tools-version:5.9
import PackageDescription

// Wraps the Rust clickr-ffi static library and its generated Swift bindings.
// Run scripts/build-ios.sh to produce ClickrFFI.xcframework and clickr_ffi.swift.
let package = Package(
    name: "ClickrKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "ClickrKit", targets: ["ClickrKit"]),
    ],
    targets: [
        .binaryTarget(name: "ClickrFFI", path: "ClickrFFI.xcframework"),
        .target(
            name: "ClickrKit",
            dependencies: ["ClickrFFI"],
            linkerSettings: [
                .linkedFramework("Security"),
                .linkedFramework("SystemConfiguration"),
                .linkedLibrary("resolv"),
            ]
        ),
    ],
    swiftLanguageVersions: [.v5]
)
