#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Document,
    Paragraph,
    Heading,
    Text,
    Strong,
    Emphasis,
    Strike,
    Link,
    InlineCode,
    Break,
    CodeBlock,
    ThematicBreak,
    List,
    ListItem,
    Quote,
    Table,
    Row,
    Cell,
}

impl Kind {
    pub fn is_inline(self) -> bool {
        matches!(
            self,
            Kind::Text | Kind::Strong | Kind::Emphasis | Kind::Strike | Kind::Link | Kind::InlineCode | Kind::Break
        )
    }

    pub fn is_mark(self) -> bool {
        matches!(self, Kind::Strong | Kind::Emphasis | Kind::Strike)
    }

    pub fn holds_inline(self) -> bool {
        matches!(self, Kind::Paragraph | Kind::Heading | Kind::Link) || self.is_mark()
    }

    pub fn is_container(self) -> bool {
        !matches!(self, Kind::Text | Kind::InlineCode | Kind::Break | Kind::CodeBlock | Kind::ThematicBreak)
    }

    pub fn is_segment(self) -> bool {
        matches!(self, Kind::Text | Kind::InlineCode | Kind::Break)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: Kind,
    pub value: String,
    pub children: Vec<Node>,
    pub href: String,
    pub title: String,
    pub level: u8,
    pub language: String,
    pub ordered: bool,
    pub start: u32,
    pub checked: Option<bool>,
    pub header: bool,
}

impl Node {
    pub fn new(kind: Kind) -> Self {
        Node {
            kind,
            value: String::new(),
            children: Vec::new(),
            href: String::new(),
            title: String::new(),
            level: 0,
            language: String::new(),
            ordered: false,
            start: 1,
            checked: None,
            header: false,
        }
    }

    pub fn parent(kind: Kind, children: Vec<Node>) -> Self {
        Node { children, ..Node::new(kind) }
    }

    pub fn leaf(kind: Kind, value: impl Into<String>) -> Self {
        Node { value: value.into(), ..Node::new(kind) }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Node::leaf(Kind::Text, value)
    }

    pub fn paragraph(children: Vec<Node>) -> Self {
        Node::parent(Kind::Paragraph, children)
    }

    pub fn code_block(value: impl Into<String>, language: String) -> Self {
        Node { language, ..Node::leaf(Kind::CodeBlock, value) }
    }

    pub fn cell(header: bool, children: Vec<Node>) -> Self {
        Node { header, ..Node::parent(Kind::Cell, children) }
    }

    pub fn link(href: String, title: String, children: Vec<Node>) -> Self {
        Node { href, title, ..Node::parent(Kind::Link, children) }
    }

    pub fn list(ordered: bool, start: u32, children: Vec<Node>) -> Self {
        Node { ordered, start, ..Node::parent(Kind::List, children) }
    }

    pub fn is(&self, kind: Kind) -> bool {
        self.kind == kind
    }

    pub fn become_text(&mut self, value: &str) {
        *self = Node::text(value);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Format,
    Length,
    Unicode,
    Structure,
    Table,
    HtmlLimit,
    Representation,
    Output,
    Unsupported,
}

impl Failure {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Failure::Format => "format",
            Failure::Length => "length",
            Failure::Unicode => "unicode",
            Failure::Structure => "structure",
            Failure::Table => "table",
            Failure::HtmlLimit => "htmlLimit",
            Failure::Representation => "representation",
            Failure::Output => "output",
            Failure::Unsupported => "unsupported",
        }
    }
}

pub type Result<T> = std::result::Result<T, Failure>;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Warnings(pub Vec<(&'static str, u32)>);

impl Warnings {
    pub fn count(&mut self, key: &'static str) {
        self.add(key, 1);
    }

    pub fn add(&mut self, key: &'static str, amount: u32) {
        match self.0.iter_mut().find(|(known, _)| *known == key) {
            Some((_, total)) => *total += amount,
            None => self.0.push((key, amount)),
        }
    }

    pub fn merge(&mut self, other: Warnings) {
        for (key, amount) in other.0 {
            self.add(key, amount);
        }
    }

    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.0.iter().any(|(known, _)| *known == key)
    }
}

pub const LIMIT: usize = 30_000;

pub fn prose(value: &str) -> Vec<Node> {
    let mut result = Vec::new();

    for (index, line) in value.split('\n').enumerate() {
        if index > 0 {
            result.push(Node::new(Kind::Break));
        }

        result.push(Node::text(line));
    }

    result
}

pub fn blocks(children: Vec<Node>) -> Vec<Node> {
    let mut result = Vec::new();
    let mut pending = Vec::new();

    for child in children {
        if child.kind.is_inline() {
            pending.push(child);
        } else {
            if !pending.is_empty() {
                result.push(Node::paragraph(std::mem::take(&mut pending)));
            }

            result.push(child);
        }
    }

    if !pending.is_empty() {
        result.push(Node::paragraph(pending));
    }

    result
}

pub fn table(rows: Vec<Vec<Node>>) -> Result<Node> {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);

    if rows.len() > 100 || width > 20 {
        return Err(Failure::Table);
    }

    let rows = rows
        .into_iter()
        .map(|mut cells| {
            cells.resize_with(width, || Node::cell(false, Vec::new()));
            Node::parent(Kind::Row, cells)
        })
        .collect();

    Ok(Node::parent(Kind::Table, rows))
}

pub fn language(value: &str) -> String {
    let name = crate::chars::trim(value).split(crate::chars::is_space).next().unwrap_or("");
    let fits = (1..=32).contains(&name.len())
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_+#.-".contains(&byte));

    if fits { name.to_owned() } else { String::new() }
}

pub fn visible(node: &Node) -> String {
    let mut out = String::new();
    visible_into(node, &mut out);
    out
}

fn visible_into(node: &Node, out: &mut String) {
    match node.kind {
        Kind::Break => out.push('\n'),
        kind if !kind.is_container() => out.push_str(&node.value),
        _ => {
            for child in &node.children {
                visible_into(child, out);
            }
        }
    }
}

pub fn segments_mut<'a>(nodes: &'a mut [Node], out: &mut Vec<&'a mut Node>) {
    for node in nodes {
        if node.kind.is_segment() {
            out.push(node);
        } else {
            segments_mut(&mut node.children, out);
        }
    }
}

pub fn segments<'a>(nodes: &'a [Node], out: &mut Vec<&'a Node>) {
    for node in nodes {
        if node.kind.is_segment() {
            out.push(node);
        } else {
            segments(&node.children, out);
        }
    }
}

pub fn check(tree: &Node) -> Result<()> {
    let mut stack = vec![(tree, 0_usize)];
    let mut size = 0;

    while let Some((node, depth)) = stack.pop() {
        size += 1;

        if size > 5000 || depth > 16 {
            return Err(Failure::Structure);
        }

        if !node.kind.is_container() && node.value.contains('\0') {
            return Err(Failure::Unicode);
        }

        let inline_parent = node.kind.holds_inline() || node.is(Kind::Cell);
        let block_parent = matches!(node.kind, Kind::Document | Kind::Quote | Kind::ListItem);

        for child in &node.children {
            let misplaced = (inline_parent && !child.kind.is_inline())
                || (node.is(Kind::List) && !child.is(Kind::ListItem))
                || (node.is(Kind::Table) && !child.is(Kind::Row))
                || (node.is(Kind::Row) && !child.is(Kind::Cell))
                || (child.is(Kind::Row) && !node.is(Kind::Table))
                || (child.is(Kind::Cell) && !node.is(Kind::Row))
                || (block_parent && child.kind.is_inline())
                || !node.kind.is_container();

            if misplaced {
                return Err(Failure::Unsupported);
            }

            stack.push((child, depth + 1));
        }
    }

    Ok(())
}
