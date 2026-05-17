import XCTest
@testable import PortholeHelper

final class SettingsLinksTests: XCTestCase {
    func testKnownPermissionHasDeepLink() {
        let link = SettingsLinks.link(for: "accessibility")
        XCTAssertEqual(link?.absoluteString, "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
    }

    func testUnknownPermissionHasNoGuessedDeepLink() {
        XCTAssertNil(SettingsLinks.link(for: "future_permission"))
    }

    func testDisplayNameFallback() {
        XCTAssertEqual(SettingsLinks.displayName(for: "screen_recording"), "Screen Recording")
        XCTAssertEqual(SettingsLinks.displayName(for: "future_permission"), "future permission")
    }
}
