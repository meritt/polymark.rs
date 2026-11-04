use crate::chars::{Kind as Class, is_sign, is_space, kind};
use crate::model::{Kind, Node, Warnings, segments_mut};

fn edge(children: &mut Vec<Node>, leading: bool) -> Vec<Node> {
    let mut moved = Vec::new();

    while !children.is_empty() {
        let position = if leading { 0 } else { children.len() - 1 };
        let node = &mut children[position];

        if node.is(Kind::Break) {
            moved.push(children.remove(position));
            continue;
        }

        if !node.is(Kind::Text) {
            break;
        }

        let split = if leading {
            node.value.len() - node.value.trim_start_matches(is_space).len()
        } else {
            node.value.trim_end_matches(is_space).len()
        };
        let space = if leading { &node.value[..split] } else { &node.value[split..] };

        if space.is_empty() {
            break;
        }

        moved.push(Node::text(space));
        node.value = if leading { node.value[split..].to_owned() } else { node.value[..split].to_owned() };

        if !node.value.is_empty() {
            break;
        }

        children.remove(position);
    }

    if !leading {
        moved.reverse();
    }

    moved
}

fn inverted(mut node: Node, neighbour: Option<Kind>) -> Node {
    let swap = node.kind.is_mark()
        && node.children.len() == 1
        && Some(node.children[0].kind) == neighbour
        && node.children[0].kind != node.kind
        && node.children[0].kind.is_mark();

    if !swap {
        return node;
    }

    let inner = node.children.remove(0);

    node.children = inner.children;
    Node::parent(inner.kind, vec![node])
}

fn canonical(mut node: Node) -> Node {
    let nested = node.is(Kind::Strong) && node.children.len() == 1 && node.children[0].is(Kind::Emphasis);

    if !nested {
        return node;
    }

    let inner = node.children.remove(0);

    Node::parent(Kind::Emphasis, vec![Node::parent(Kind::Strong, inner.children)])
}

fn flatten(children: Vec<Node>, kind: Kind, out: &mut Vec<Node>) {
    for child in children {
        if child.kind == kind {
            flatten(child.children, kind, out);
        } else {
            out.push(child);
        }
    }
}

fn bit(kind: Kind) -> u8 {
    match kind {
        Kind::Strong => 1,
        Kind::Emphasis => 2,
        _ => 4,
    }
}

fn append(result: &mut Vec<Node>, next: Node, warnings: &mut Warnings) {
    let node = inverted(next, result.last().map(|last| last.kind));

    if let Some(last) = result.last_mut() {
        let previous = std::mem::replace(last, Node::new(Kind::Break));
        *last = inverted(previous, Some(node.kind));
    }

    let leaf = matches!(node.kind, Kind::Text | Kind::InlineCode);

    if leaf && node.value.is_empty() {
        return;
    }

    match result.last_mut() {
        Some(previous) if previous.kind == node.kind && leaf => previous.value.push_str(&node.value),
        Some(previous) if previous.kind == node.kind && node.kind.is_mark() => {
            previous.children.extend(node.children);
            normalize_within(previous, warnings, 0);
        }
        _ => result.push(node),
    }
}

pub fn normalize(tree: &mut Node, warnings: &mut Warnings) {
    normalize_within(tree, warnings, 0);
}

fn normalize_within(tree: &mut Node, warnings: &mut Warnings, ancestors: u8) {
    if !tree.kind.is_container() {
        return;
    }

    let mut result = Vec::new();

    for mut node in std::mem::take(&mut tree.children) {
        let mark = node.kind.is_mark();

        if mark {
            let mut flat = Vec::new();
            flatten(std::mem::take(&mut node.children), node.kind, &mut flat);
            node.children = flat;
        }

        let within = if mark { ancestors | bit(node.kind) } else { ancestors };

        normalize_within(&mut node, warnings, within);

        if mark && ancestors & bit(node.kind) != 0 {
            for child in node.children {
                append(&mut result, child, warnings);
            }
        } else if node.is(Kind::Link) || mark {
            let before = edge(&mut node.children, true);
            let after = edge(&mut node.children, false);

            if mark && (!before.is_empty() || !after.is_empty()) {
                warnings.count("emphasis");
            }

            for child in before {
                append(&mut result, child, warnings);
            }

            if !node.children.is_empty() {
                append(&mut result, if mark { canonical(node) } else { node }, warnings);
            }

            for child in after {
                append(&mut result, child, warnings);
            }
        } else {
            append(&mut result, node, warnings);
        }
    }

    result.retain(|node| !node.kind.holds_inline() || !node.children.is_empty());
    tree.children = result;
}

fn delimiter(kind: Kind) -> Option<char> {
    match kind {
        Kind::Strong | Kind::Emphasis => Some('*'),
        Kind::Strike => Some('~'),
        _ => None,
    }
}

fn side(node: &Node, last: bool) -> Option<char> {
    match node.kind {
        Kind::Text => {
            if last {
                node.value.chars().next_back()
            } else {
                node.value.chars().next()
            }
        }
        Kind::Break => Some('\n'),
        Kind::InlineCode => Some('`'),
        Kind::Link => Some(if last { ')' } else { '[' }),
        kind => delimiter(kind),
    }
}

pub fn marker(parent: Kind, siblings: &[Node], index: usize) -> Option<char> {
    let node = &siblings[index];
    let before = index.checked_sub(1).and_then(|previous| siblings.get(previous));
    let near = before.into_iter().chain(siblings.get(index + 1)).chain(&node.children);
    let mixed = parent == Kind::Strong || near.into_iter().any(|item| item.is(Kind::Strong));

    if node.is(Kind::Emphasis) && mixed { Some('_') } else { delimiter(node.kind) }
}

pub fn delimits(sign: char, before: Class, after: Class) -> (bool, bool) {
    let left = after != Class::Space && (after != Class::Punctuation || before != Class::Word);
    let right = before != Class::Space && (before != Class::Punctuation || after != Class::Word);

    if sign != '_' {
        return (left, right);
    }

    (left && (!right || before == Class::Punctuation), right && (!left || after == Class::Punctuation))
}

fn edge_of(item: &Node, last: bool) -> Option<&Node> {
    if last { item.children.last() } else { item.children.first() }
}

fn flanks(parent: &Node, index: usize, last: bool) -> bool {
    let node = &parent.children[index];
    let sign = marker(parent.kind, &parent.children, index);
    let mut child = edge_of(node, last);

    while let Some(current) = child {
        if delimiter(current.kind).is_none() || delimiter(current.kind) != sign || current.children.is_empty() {
            break;
        }

        child = edge_of(current, last);
    }

    let neighbour =
        if last { parent.children.get(index + 1) } else { index.checked_sub(1).map(|at| &parent.children[at]) };
    let boundary = match parent.kind {
        Kind::Strike => Some('~'),
        Kind::Strong | Kind::Emphasis => Some('*'),
        Kind::Link => Some('['),
        _ => Some(' '),
    };
    let inside = child.and_then(|child| side(child, last));
    let outside = match neighbour {
        Some(neighbour) => side(neighbour, !last),
        None => boundary,
    };
    let sign = sign.unwrap_or('*');

    [Class::Word, Class::Punctuation].into_iter().all(|tilde| {
        let classify = |char: Option<char>| if char == Some('~') { tilde } else { kind(char) };
        let inner = classify(inside);
        let outer = classify(outside);

        if last { delimits(sign, inner, outer).1 } else { delimits(sign, outer, inner).0 }
    })
}

fn release(parent: &mut Node, index: usize, last: bool, warnings: &mut Warnings) {
    let node = &mut parent.children[index];
    let child = if last { node.children.last_mut() } else { node.children.first_mut() };
    let signs = child.filter(|child| child.is(Kind::Text)).and_then(|child| {
        let split = if last {
            child.value.trim_end_matches(is_sign).len()
        } else {
            child.value.len() - child.value.trim_start_matches(is_sign).len()
        };
        let signs = if last { child.value[split..].to_owned() } else { child.value[..split].to_owned() };

        if signs.is_empty() {
            return None;
        }

        child.value = if last { child.value[..split].to_owned() } else { child.value[split..].to_owned() };
        Some(signs)
    });

    match signs {
        None => {
            warnings.count("marks");
            let node = parent.children.remove(index);
            parent.children.splice(index..index, node.children);
        }
        Some(signs) => {
            parent.children.insert(if last { index + 1 } else { index }, Node::text(signs));
        }
    }
}

fn repair(tree: &mut Node, warnings: &mut Warnings) -> bool {
    let mut changed = false;
    let mut index = 0;

    while index < tree.children.len() {
        if repair(&mut tree.children[index], warnings) {
            changed = true;
        }

        let node = &tree.children[index];

        if node.kind.is_mark() && !node.children.is_empty() {
            for last in [false, true] {
                if !tree.children[index].children.is_empty() && !flanks(tree, index, last) {
                    release(tree, index, last, warnings);
                    changed = true;
                    break;
                }
            }
        }

        index += 1;
    }

    changed
}

pub fn repair_marks(tree: &mut Node, warnings: &mut Warnings) {
    for _ in 0..64 {
        if !repair(tree, warnings) {
            break;
        }

        normalize(tree, warnings);
    }
}

pub fn unmark(tree: &mut Node) {
    if tree.kind.is_container() {
        for mut child in std::mem::take(&mut tree.children) {
            unmark(&mut child);

            if child.kind.is_mark() {
                tree.children.append(&mut child.children);
            } else {
                tree.children.push(child);
            }
        }
    }

    normalize(tree, &mut Warnings::default());
}

fn flatten_table(node: &mut Node, warnings: &mut Warnings) {
    for (y, row) in node.children.iter_mut().enumerate() {
        for item in &mut row.children {
            if item.header && y > 0 {
                warnings.count("cells");
            }

            item.header = y == 0;

            let mut flow = Vec::new();
            segments_mut(&mut item.children, &mut flow);

            for part in flow {
                let lines = part.value.contains(['\r', '\n']);

                if part.is(Kind::Break) {
                    warnings.count("cells");
                    part.become_text(" ");
                } else if part.is(Kind::InlineCode) && lines {
                    warnings.count("cells");
                    part.value = squash_lines(&part.value);
                } else if part.is(Kind::InlineCode) && part.value.is_empty() {
                    part.become_text("");
                }
            }
        }
    }
}

fn squash_lines(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut broken = false;

    for char in value.chars() {
        let line = matches!(char, '\r' | '\n');

        if !line {
            result.push(char);
        } else if !broken {
            result.push(' ');
        }

        broken = line;
    }

    result
}

pub fn prepare_markdown(tree: &mut Node, warnings: &mut Warnings) {
    if tree.is(Kind::Table) {
        flatten_table(tree, warnings);
        return;
    }

    if !tree.kind.is_container() {
        return;
    }

    if tree.is(Kind::Heading) {
        let mut flow = Vec::new();
        segments_mut(&mut tree.children, &mut flow);

        for part in flow {
            if part.is(Kind::Break) {
                warnings.count("headings");
                part.become_text(" ");
            }
        }
    }

    for child in &mut tree.children {
        prepare_markdown(child, warnings);
    }

    for child in &mut tree.children {
        if child.is(Kind::InlineCode) && child.value.contains(['\r', '\n']) {
            warnings.count("codeBlock");
            *child = Node::code_block(std::mem::take(&mut child.value), String::new());
        }
    }

    let mut children = Vec::new();

    for mut child in std::mem::take(&mut tree.children) {
        if !child.kind.is_container() || !child.kind.holds_inline() {
            children.push(child);
            continue;
        }

        let mut pending = Vec::new();

        for item in std::mem::take(&mut child.children) {
            if item.is(Kind::CodeBlock) {
                if !pending.is_empty() {
                    children.push(Node { children: std::mem::take(&mut pending), ..child.clone() });
                }

                children.push(item);
            } else {
                pending.push(item);
            }
        }

        if !pending.is_empty() {
            children.push(Node { children: pending, ..child });
        }
    }

    tree.children = children;
}

pub fn losses(tree: &Node, plain: bool, warnings: &mut Warnings) {
    if plain && tree.kind.is_mark() {
        warnings.count("formatting");
    }

    if plain && tree.is(Kind::Table) {
        warnings.count("tables");
    }

    for child in &tree.children {
        losses(child, plain, warnings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(kind: Kind, children: Vec<Node>) -> Node {
        Node::parent(kind, children)
    }

    #[test]
    fn merges_and_moves_spaces() {
        let mut tree = Node::paragraph(vec![
            mark(Kind::Strong, vec![Node::text("a ")]),
            mark(Kind::Strong, vec![Node::text("b")]),
            Node::text(""),
            mark(Kind::Emphasis, vec![Node::text(" c")]),
        ]);
        let mut warnings = Warnings::default();

        normalize(&mut tree, &mut warnings);

        assert_eq!(
            tree.children,
            vec![
                mark(Kind::Strong, vec![Node::text("a")]),
                Node::text(" "),
                mark(Kind::Strong, vec![Node::text("b")]),
                Node::text(" "),
                mark(Kind::Emphasis, vec![Node::text("c")])
            ]
        );
        assert!(warnings.has("emphasis"));
    }

    #[test]
    fn releases_punctuation() {
        let mut tree =
            Node::paragraph(vec![Node::text("a"), mark(Kind::Strong, vec![Node::text("«b»")]), Node::text("c")]);
        let mut warnings = Warnings::default();

        repair_marks(&mut tree, &mut warnings);

        assert_eq!(tree.children, vec![Node::text("a«"), mark(Kind::Strong, vec![Node::text("b")]), Node::text("»c")]);
    }
}
