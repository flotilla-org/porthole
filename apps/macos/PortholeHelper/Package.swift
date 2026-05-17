// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "PortholeHelper",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "PortholeHelper", targets: ["PortholeHelper"]),
    ],
    targets: [
        .executableTarget(name: "PortholeHelper"),
    ]
)
