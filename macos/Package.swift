// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "Chatkeep",
    platforms: [.macOS(.v15)],
    products: [
        .executable(name: "Chatkeep", targets: ["Chatkeep"]),
        .library(name: "ChatkeepKit", targets: ["ChatkeepKit"]),
    ],
    targets: [
        .target(name: "ChatkeepKit"),
        .executableTarget(
            name: "Chatkeep",
            dependencies: ["ChatkeepKit"],
            swiftSettings: [.defaultIsolation(MainActor.self)]
        ),
        .testTarget(
            name: "ChatkeepKitTests",
            dependencies: ["ChatkeepKit"],
            resources: [.copy("Fixtures")]
        ),
    ]
)
