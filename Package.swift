// swift-tools-version: 5.9
import PackageDescription
let package = Package(
    name: "NexusCore", platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "NexusCore", targets: ["NexusCore"])],
    targets: [.target(name: "NexusCore"), .testTarget(name: "NexusCoreTests", dependencies: ["NexusCore"])]
)
