mod chars;
mod clean;
mod dom;
mod html;
mod markdown;
mod model;
mod normalize;
mod signs;
mod telegram;
mod write;

use std::fmt::Write;

use chars::{is_blank, utf16_len};
use clean::{clean_plain, clean_tree, plain_tree, trim_lines, unicode_warnings, unlink_addresses};
use normalize::{losses, normalize, prepare_markdown, repair_marks, unmark};
use write::{Format, serialize};

pub use model::{Failure, LIMIT, Result, Warnings};
use model::{Kind, Node};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Plain,
    Markdown,
    Html,
    Rich,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Plain,
    Markdown,
    Html,
    Rich,
    Telegram,
}

impl Input {
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "plain" => Some(Input::Plain),
            "markdown" => Some(Input::Markdown),
            "html" => Some(Input::Html),
            "rich" => Some(Input::Rich),
            _ => None,
        }
    }
}

impl Output {
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "plain" => Some(Output::Plain),
            "markdown" => Some(Output::Markdown),
            "html" => Some(Output::Html),
            "rich" => Some(Output::Rich),
            "telegram" => Some(Output::Telegram),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub result: String,
    pub plain: Option<String>,
    pub warnings: Warnings,
}

fn validate(source: &str) -> Result<()> {
    if source.is_empty() || utf16_len(source) > LIMIT {
        return Err(Failure::Length);
    }

    if source.contains('\0') {
        return Err(Failure::Unicode);
    }

    Ok(())
}

fn tree(value: &str, from: Input, warnings: &mut Warnings) -> Result<Node> {
    match from {
        Input::Plain => Ok(plain_tree(&clean_plain(value), warnings)),
        Input::Markdown => markdown::parse(value, warnings),
        Input::Html | Input::Rich => html::parse(value, html::BOUNDS, warnings),
    }
}

fn found(tree: &Node, warnings: &mut Warnings) {
    let mut stack = vec![tree];

    while let Some(node) = stack.pop() {
        if !node.kind.is_container() && node.kind != Kind::Break && node.kind != Kind::ThematicBreak {
            unicode_warnings(&node.value, warnings);
        }

        stack.extend(&node.children);
    }
}

fn comparable(tree: &Node) -> Node {
    let mut copy = tree.clone();

    unlink_addresses(&mut copy);
    normalize(&mut copy, &mut Warnings::default());
    copy
}

fn markdown(document: &mut Node, warnings: &mut Warnings) -> Result<String> {
    for plain in [false, true] {
        if plain {
            unmark(document);
            warnings.count("unmarked");
        }

        let expected = comparable(document);

        for strict in [false, true] {
            let result = serialize(document, Format::Markdown, warnings, strict)?;

            if comparable(&markdown::parse(&result, &mut Warnings::default())?) == expected {
                return Ok(result);
            }
        }
    }

    Err(Failure::Representation)
}

fn convert(source: &str, from: Input, to: Output) -> Result<Converted> {
    let mut warnings = Warnings::default();

    validate(source)?;

    let value = source.replace("\r\n", "\n").replace('\r', "\n");

    if from == Input::Plain && to == Output::Plain {
        unicode_warnings(&value, &mut warnings);

        return Ok(Converted { result: clean_plain(&value), plain: None, warnings });
    }

    let mut document = tree(&value, from, &mut warnings)?;

    found(&document, &mut warnings);

    if from == Input::Html {
        clean_tree(&mut document);
    }

    if to == Output::Markdown {
        prepare_markdown(&mut document, &mut warnings);
    }

    normalize(&mut document, &mut warnings);

    if to == Output::Markdown {
        trim_lines(&mut document);
        normalize(&mut document, &mut warnings);
        repair_marks(&mut document, &mut warnings);
    }

    losses(&document, to == Output::Plain, &mut warnings);

    let (result, plain) = match to {
        Output::Telegram => (telegram::telegram(&document, &mut warnings)?, None),
        Output::Markdown => (markdown(&mut document, &mut warnings)?, None),
        Output::Rich => {
            let plain = serialize(&document, Format::Plain, &mut Warnings::default(), false)?;

            (serialize(&document, Format::Html, &mut warnings, false)?, Some(plain))
        }
        Output::Plain => (serialize(&document, Format::Plain, &mut warnings, false)?, None),
        Output::Html => (serialize(&document, Format::Html, &mut warnings, false)?, None),
    };

    Ok(Converted { result, plain, warnings })
}

pub fn clean(source: &str) -> Result<String> {
    if utf16_len(source) > 2_000_000 {
        return Err(Failure::Length);
    }

    let bounds = html::Bounds { tags: 50_000, nodes: 100_000 };
    let mut document = html::parse(source, bounds, &mut Warnings::default())?;

    clean_tree(&mut document);
    normalize(&mut document, &mut Warnings::default());
    serialize(&document, Format::Html, &mut Warnings::default(), false)
}

pub fn transform(source: &str, from: Input, to: Output) -> Result<Converted> {
    let mut converted =
        if from == Input::Rich { convert(&clean(source)?, Input::Html, to)? } else { convert(source, from, to)? };

    if utf16_len(&converted.result) > LIMIT * 5 {
        return Err(Failure::Output);
    }

    if is_blank(&converted.result) {
        converted.result.clear();
    }

    Ok(converted)
}

fn quote(value: &str, out: &mut String) {
    out.push('"');

    for char in value.chars() {
        match char {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            char if (char as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", char as u32);
            }
            char => out.push(char),
        }
    }

    out.push('"');
}

#[must_use]
pub fn report(outcome: &Result<Converted>) -> String {
    let mut out = String::new();

    match outcome {
        Err(failure) => {
            out.push_str("{\"error\":");
            quote(failure.code(), &mut out);
        }
        Ok(converted) => {
            out.push_str("{\"result\":");
            quote(&converted.result, &mut out);

            if let Some(plain) = &converted.plain {
                out.push_str(",\"plain\":");
                quote(plain, &mut out);
            }

            out.push_str(",\"warnings\":{");

            for (index, (key, amount)) in converted.warnings.0.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }

                quote(key, &mut out);
                let _ = write!(out, ":{amount}");
            }

            out.push('}');
        }
    }

    out.push('}');
    out
}

#[must_use]
pub fn report_clean(outcome: &Result<String>) -> String {
    let mut out = String::new();

    match outcome {
        Err(failure) => {
            out.push_str("{\"error\":");
            quote(failure.code(), &mut out);
        }
        Ok(result) => {
            out.push_str("{\"result\":");
            quote(result, &mut out);
        }
    }

    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, from: &str, to: &str) -> String {
        transform(source, Input::parse(from).unwrap(), Output::parse(to).unwrap())
            .map_or_else(|failure| format!("!{}", failure.code()), |converted| converted.result)
    }

    #[test]
    fn markdown_round_trips() {
        assert_eq!(
            run("**a** _b_ ~~c~~ `d`\n\n- [x] e\n\n1. f", "markdown", "markdown"),
            "**a** *b* ~~c~~ `d`\n\n- [x] e\n\n1. f"
        );
    }

    #[test]
    fn html_to_markdown() {
        assert_eq!(
            run(
                "<h2>T</h2><p>a <b>b</b> <a href=\"https://e.com\">e</a></p><table><tr><th>x</th></tr><tr><td>1</td></tr></table>",
                "html",
                "markdown"
            ),
            "## T\n\na **b** [e](https://e.com)\n\n| x |\n| --- |\n| 1 |"
        );
    }

    #[test]
    fn markdown_to_html() {
        assert_eq!(
            run("# T\n\na\nb https://e.com\n\n```js\nx\n```", "markdown", "html"),
            "<h1>T</h1>\n<p>a<br>b <a href=\"https://e.com\">https://e.com</a></p>\n<pre><code class=\"language-js\">x</code></pre>"
        );
    }

    #[test]
    fn plain_and_telegram() {
        assert_eq!(run("a  b\n\n\n\nc ", "plain", "plain"), "a b\n\nc");
        assert_eq!(run("**a.b** [c](https://e.com)", "markdown", "telegram"), "*a\\.b* [c](https://e.com)");
    }

    #[test]
    fn reports_failures() {
        assert_eq!(run("", "plain", "html"), "!length");
        assert_eq!(report(&Err(Failure::Table)), "{\"error\":\"table\"}");
    }
}
