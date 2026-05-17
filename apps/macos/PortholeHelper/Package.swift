// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "PortholeHelper",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "PortholeHelper", targets: ["PortholeHelper"]),
    ],
    targets: [
        .executableTarget(name: "PortholeHelper"),
        .testTarget(name: "PortholeHelperTests", dependencies: ["PortholeHelper"]),
    ]
)
