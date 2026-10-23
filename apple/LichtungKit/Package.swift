// swift-tools-version: 6.0
import PackageDescription

// LichtungKit ist der gemeinsame Kern der Mac- und iPhone-App (Spec §3):
// Modelle, Satz-Parser, Cache, Sync-Client, Serienexpansion.
// Bewusst nur Foundation, kein SwiftUI/UIKit/AppKit: `swift test` läuft
// damit auch unter Linux (CI) und ohne Xcode-Simulator.
let package = Package(
    name: "LichtungKit",
    platforms: [
        .iOS(.v18),
        .macOS(.v15),
    ],
    products: [
        .library(name: "LichtungKit", targets: ["LichtungKit"]),
    ],
    targets: [
        .target(
            name: "LichtungKit",
            swiftSettings: [
                .swiftLanguageMode(.v6),
            ]
        ),
        .testTarget(
            name: "LichtungKitTests",
            dependencies: ["LichtungKit"],
            swiftSettings: [
                .swiftLanguageMode(.v6),
            ]
        ),
    ]
)
