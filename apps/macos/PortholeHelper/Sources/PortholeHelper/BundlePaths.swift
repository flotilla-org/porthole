import Foundation

struct BundlePaths {
    let bundleURL: URL

    static func current() -> BundlePaths {
        BundlePaths(bundleURL: Bundle.main.bundleURL)
    }

    var contentsURL: URL {
        bundleURL.appendingPathComponent("Contents", isDirectory: true)
    }

    var macOSURL: URL {
        contentsURL.appendingPathComponent("MacOS", isDirectory: true)
    }

    var cliURL: URL {
        macOSURL.appendingPathComponent("porthole")
    }
}
