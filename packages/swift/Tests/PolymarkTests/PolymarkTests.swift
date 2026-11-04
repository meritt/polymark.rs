import Foundation
import Testing

@testable import Polymark

private struct Want: Decodable {
    let result: String?
    let plain: String?
    let warnings: [String: Int]?
    let error: String?
}

private struct Case: Decodable {
    let name: String
    let from: String?
    let to: String?
    let input: String
    let repeatCount: Int?
    let want: Want
    enum CodingKeys: String, CodingKey {
        case name, from, to, input, want
        case repeatCount = "repeat"
    }

    var text: String {
        String(repeating: input, count: repeatCount ?? 1)
    }
}

private struct Corpus: Decodable {
    let format: Int
    let transform: [Case]
    let clean: [Case]
}

private let corpusURL: URL = {
    if let path = ProcessInfo.processInfo.environment["POLYMARK_CORPUS"] {
        return URL(fileURLWithPath: path)
    }
    return URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .appendingPathComponent("../../../../testdata/cases.json")
        .standardized
}()

private func loadCorpus() throws -> Corpus {
    let corpus = try JSONDecoder().decode(Corpus.self, from: Data(contentsOf: corpusURL))
    #expect(corpus.format == 1)
    return corpus
}

private func format(_ name: String?) -> Format? {
    Format.allCases.first { "\($0)" == name }
}

@Test func transformCorpusCases() throws {
    let cases = try loadCorpus().transform
    #expect(!cases.isEmpty)
    for c in cases {
        let from = try #require(format(c.from), "\(c.name)")
        let to = try #require(format(c.to), "\(c.name)")
        let want = c.want
        do {
            let converted = try Polymark.transform(c.text, from: from, to: to)
            let expected = Converted(result: want.result ?? "", plain: want.plain, warnings: want.warnings ?? [:])
            #expect(want.error == nil, "\(c.name)")
            #expect(converted == expected, "\(c.name)")
        } catch {
            #expect(error.code == want.error, "\(c.name)")
        }
    }
}

@Test func cleanCorpusCases() throws {
    let cases = try loadCorpus().clean
    #expect(!cases.isEmpty)
    for c in cases {
        do {
            #expect(try Polymark.clean(c.text) == c.want.result, "\(c.name)")
        } catch {
            #expect(error.code == c.want.error, "\(c.name)")
        }
    }
}

@Test func reportsTheVersion() {
    #expect(!Polymark.version.isEmpty)
}

@Test func refusesAnOutputAsInput() {
    #expect(throws: PolymarkError(code: "invalid_argument")) { try Polymark.transform("a", from: .telegram, to: .html) }
}
