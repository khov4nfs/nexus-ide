import Foundation

/// One reviewed plain-text edit in the saved snapshot, using LSP UTF-16 coordinates.
/// No commands, additional edits or snippet evaluation.
public struct LSPTextEdit: Codable, Sendable {
    public struct Position: Codable, Sendable {
        public let line: Int
        public let character: Int
        public init(line: Int, character: Int) { self.line = line; self.character = character }
    }
    public struct Range: Codable, Sendable {
        public let start: Position
        public let end: Position
        public init(start: Position, end: Position) { self.start = start; self.end = end }
    }
    public let label: String
    public let text: String
    public let range: Range
    public init(label: String, text: String, range: Range) {
        self.label = label; self.text = text; self.range = range
    }
    public static func offset(_ position: Position, in source: String) -> Int? {
        guard position.line >= 0, position.character >= 0 else { return nil }
        let units = Array(source.utf16)
        var start = 0
        for _ in 0..<min(position.line, units.count + 1) {
            guard let newline = units[start...].firstIndex(of: 10) else { return nil }
            start = newline + 1
        }
        guard position.line <= units.count + 1 else { return nil }
        var end = units[start...].firstIndex(of: 10) ?? units.count
        if end > start, units[end - 1] == 13 { end -= 1 }
        guard position.character <= end - start else { return nil }
        let offset = start + position.character
        if offset > 0, offset < units.count, (0xD800...0xDBFF).contains(units[offset-1]), (0xDC00...0xDFFF).contains(units[offset]) { return nil }
        return offset
    }
    public func replacement(in source: String) throws -> String {
        guard source.utf8.count <= 8192, !source.utf8.contains(0), !text.isEmpty, text.utf8.count <= 1024,
              !text.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
              range.start.line == range.end.line,
              let start = Self.offset(range.start, in: source), let end = Self.offset(range.end, in: source), start <= end else { throw WorkspaceError.invalidPath }
        var units = Array(source.utf16)
        units.replaceSubrange(start..<end, with: text.utf16)
        let result = String(decoding: units, as: UTF16.self)
        guard result.utf8.count <= 8192 else { throw WorkspaceError.tooLarge }
        return result
    }
}
