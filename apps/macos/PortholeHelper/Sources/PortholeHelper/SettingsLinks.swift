import Foundation

enum SettingsLinks {
    static func link(for permission: String) -> URL? {
        switch permission {
        case "accessibility":
            URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        case "screen_recording":
            URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        default:
            nil
        }
    }

    static func displayName(for permission: String) -> String {
        switch permission {
        case "accessibility":
            "Accessibility"
        case "screen_recording":
            "Screen Recording"
        default:
            permission.replacingOccurrences(of: "_", with: " ")
        }
    }
}
