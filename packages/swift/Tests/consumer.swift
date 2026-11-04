import Polymark

let converted = try Polymark.transform("**a**", from: .markdown, to: .html)
precondition(converted.result == "<p><strong>a</strong></p>")
