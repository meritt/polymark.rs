// swift-tools-version: 6.2
import Foundation
import PackageDescription

let release: (url: String, checksum: String)? = nil

let local = "packages/swift/PolymarkFFI.xcframework"
let ffi: Target
if FileManager.default.fileExists(atPath: Context.packageDirectory + "/" + local) {
    ffi = .binaryTarget(name: "PolymarkFFI", path: local)
} else if let release {
    ffi = .binaryTarget(name: "PolymarkFFI", url: release.url, checksum: release.checksum)
} else {
    fatalError("no released PolymarkFFI yet: build \(local) with `cargo xtask xcframework`")
}

let package = Package(
    name: "Polymark",
    platforms: [.macOS(.v26)],
    products: [
        .library(name: "Polymark", targets: ["Polymark"]),
    ],
    targets: [
        ffi,
        .target(name: "Polymark", dependencies: ["PolymarkFFI"], path: "packages/swift/Sources/Polymark"),
        .testTarget(name: "PolymarkTests", dependencies: ["Polymark"], path: "packages/swift/Tests/PolymarkTests"),
    ]
)
