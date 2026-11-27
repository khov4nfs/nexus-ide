import Foundation

public enum GeminiCredential {
    /// Shape check only. Actual authentication requires the official agent.
    public static func isValid(_ key: String) -> Bool {
        (16...256).contains(key.utf8.count) && key.utf8.allSatisfy {
            (48...57).contains($0) || (65...90).contains($0) ||
            (97...122).contains($0) || $0 == 45 || $0 == 95
        }
    }
}
