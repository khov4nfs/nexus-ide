import Foundation

public enum AGYImportFormat {
    public static let maxBytes: UInt64 = 1024 * 1024 * 1024
    public static let slot = Data("NEXUS_AGY_IMPORT_SLOT_V1\n".utf8)
    public static func validDigest(_ sha: String) -> Bool {
        sha.utf8.count == 64 && sha.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
    public static func validHeader(_ data: Data) -> Bool {
        let b = Array(data.prefix(64))
        guard b.count == 64, Array(b.prefix(7)) == [0x7f, 0x45, 0x4c, 0x46, 2, 1, 1] else { return false }
        return (b[16] == 2 || b[16] == 3) && b[17] == 0 && b[18] == 183 && b[19] == 0 &&
            Array(b[20..<24]) == [1, 0, 0, 0]
    }
}
