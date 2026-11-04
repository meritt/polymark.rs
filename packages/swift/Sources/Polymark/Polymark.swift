import Foundation
import PolymarkFFI

public enum Polymark {
    public static let version = String(cString: polymark_version())

    public static func transform(_ text: String, from: Format, to: Format) throws(PolymarkError) -> Converted {
        let input = Array(text.utf8)
        let json = try report { out in
            input.withUnsafeBufferPointer { input in
                polymark_transform(input.baseAddress, input.count, from.rawValue, to.rawValue, out)
            }
        }
        return try decode(Converted.self, json)
    }

    public static func clean(_ html: String) throws(PolymarkError) -> String {
        let input = Array(html.utf8)
        let json = try report { out in
            input.withUnsafeBufferPointer { input in
                polymark_clean(input.baseAddress, input.count, out)
            }
        }
        return try decode(Cleaned.self, json).result
    }

    private static func report(_ call: (UnsafeMutablePointer<polymark_report>) -> Void) throws(PolymarkError) -> Data {
        var out = polymark_report()
        call(&out)
        defer { polymark_report_free(&out) }
        switch out.status {
        case UInt32(POLYMARK_OK):
            return Data(UnsafeBufferPointer(start: out.data, count: out.len))
        case UInt32(POLYMARK_INVALID_UTF8):
            throw PolymarkError(code: "invalid_utf8")
        case UInt32(POLYMARK_INVALID_ARGUMENT):
            throw PolymarkError(code: "invalid_argument")
        default:
            throw PolymarkError(code: "internal")
        }
    }

    private static func decode<T: Decodable>(_ type: T.Type, _ json: Data) throws(PolymarkError) -> T {
        let decoder = JSONDecoder()
        if let failure = try? decoder.decode(Failure.self, from: json) {
            throw PolymarkError(code: failure.error)
        }
        do {
            return try decoder.decode(type, from: json)
        } catch {
            throw PolymarkError(code: "internal")
        }
    }
}

public enum Format: UInt32, CaseIterable, Sendable {
    case plain
    case markdown
    case html
    case rich
    case telegram
}

public struct Converted: Decodable, Equatable, Sendable {
    public let result: String
    public let plain: String?
    public let warnings: [String: Int]
}

public struct PolymarkError: Error, Equatable, Sendable {
    public let code: String
}

private struct Cleaned: Decodable {
    let result: String
}

private struct Failure: Decodable {
    let error: String
}
