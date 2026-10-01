// SPDX-License-Identifier: GPL-3.0-or-later
// A deliberately tiny KDL reader/writer for the niri `input` blocks the
// Settings panels edit (keyboard, touchpad). It understands one node per line,
// `name arg arg`, `name {` … `}` and `name {}`; quoted strings, bare numbers and
// bare words as arguments; properties with a bare value (`horizontal=-1.0`,
// kept as one argument); `//` comments. Anything else (quoted property values,
// `;`, slashdash, multi-line strings) is a parse error, so a hand-edited region
// we can't faithfully round-trip is left alone rather than mangled.
//
// Panels edit the tree (set/remove the nodes they own) and render it back, so
// every setting they don't know about survives untouched. Comments inside a
// managed region are not preserved.

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub text: String,
    pub quoted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: String,
    pub args: Vec<Arg>,
    pub children: Option<Vec<Node>>,
}

impl Node {
    pub fn flag(name: &str) -> Node {
        Node { name: name.into(), args: vec![], children: None }
    }
    pub fn str(name: &str, v: &str) -> Node {
        Node { name: name.into(), args: vec![Arg { text: v.into(), quoted: true }], children: None }
    }
    pub fn num(name: &str, v: &str) -> Node {
        Node { name: name.into(), args: vec![Arg { text: v.into(), quoted: false }], children: None }
    }
    pub fn block(name: &str, children: Vec<Node>) -> Node {
        Node { name: name.into(), args: vec![], children: Some(children) }
    }
}

/// Cut a trailing `//` comment, ignoring `//` inside a quoted string.
fn strip_comment(line: &str) -> &str {
    let mut in_q = false;
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => in_q = !in_q,
            b'/' if !in_q && b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

fn tokens(line: &str) -> Result<Vec<Arg>, String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut s = String::new();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some('\\') => return Err("escape sequences in strings are not supported".into()),
                    Some(ch) => s.push(ch),
                    None => return Err("unterminated string".into()),
                }
            }
            out.push(Arg { text: s, quoted: true });
        } else {
            let mut s = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_whitespace() || ch == '"' {
                    break;
                }
                s.push(ch);
                chars.next();
            }
            out.push(Arg { text: s, quoted: false });
        }
    }
    Ok(out)
}

/// A bare token is fine unless it holds `=`, which only a `key=value`
/// property with a bare value may.
fn prop_ok(t: &str) -> bool {
    match t.split_once('=') {
        None => true,
        Some((k, v)) => !k.is_empty() && !v.is_empty() && !v.contains('=') && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
    }
}

/// The value of property `key` (`key=value`) among a node's arguments.
pub fn prop<'a>(n: &'a Node, key: &str) -> Option<&'a str> {
    n.args.iter().filter(|a| !a.quoted).find_map(|a| a.text.strip_prefix(key)?.strip_prefix('='))
}

pub fn parse(body: &str) -> Result<Vec<Node>, String> {
    // Stack of (node-being-built); the bottom frame is the root list.
    let mut stack: Vec<Vec<Node>> = vec![Vec::new()];
    let mut open: Vec<Node> = Vec::new();
    for (n, raw) in body.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let at = |m: &str| format!("line {}: {m}", n + 1);
        if line == "}" {
            let done = stack.pop().filter(|_| !open.is_empty()).ok_or_else(|| at("unmatched }"))?;
            let mut node = open.pop().ok_or_else(|| at("unmatched }"))?;
            node.children = Some(done);
            stack.last_mut().ok_or_else(|| at("unmatched }"))?.push(node);
            continue;
        }
        let mut toks = tokens(line).map_err(|e| at(&e))?;
        let name = toks.remove(0);
        if name.quoted || name.text.contains(['=', ';', '{', '}']) || name.text.starts_with("/-") {
            return Err(at("unsupported syntax"));
        }
        let mut opens = false;
        let mut empty_block = false;
        match toks.last() {
            Some(t) if !t.quoted && t.text == "{" => {
                opens = true;
                toks.pop();
            }
            Some(t) if !t.quoted && t.text == "{}" => {
                empty_block = true;
                toks.pop();
            }
            _ => {}
        }
        if toks.iter().any(|t| !t.quoted && t.text.contains([';', '{', '}']) || !t.quoted && !prop_ok(&t.text)) {
            return Err(at("unsupported syntax (properties or multiple nodes per line)"));
        }
        let node = Node {
            name: name.text,
            args: toks,
            children: if empty_block { Some(vec![]) } else { None },
        };
        if opens {
            open.push(node);
            stack.push(Vec::new());
        } else {
            stack.last_mut().unwrap().push(node);
        }
    }
    if !open.is_empty() {
        return Err("unclosed {".into());
    }
    Ok(stack.pop().unwrap_or_default())
}

fn render_node(n: &Node, depth: usize, out: &mut Vec<String>) {
    let pad = "    ".repeat(depth);
    let args: String = n
        .args
        .iter()
        .map(|a| if a.quoted { format!(" \"{}\"", a.text) } else { format!(" {}", a.text) })
        .collect();
    match &n.children {
        None => out.push(format!("{pad}{}{args}", n.name)),
        Some(c) if c.is_empty() => out.push(format!("{pad}{}{args} {{}}", n.name)),
        Some(c) => {
            out.push(format!("{pad}{}{args} {{", n.name));
            for ch in c {
                render_node(ch, depth + 1, out);
            }
            out.push(format!("{pad}}}"));
        }
    }
}

/// Render at `depth` (the region sits inside `input {}`, so depth 1).
pub fn render(nodes: &[Node], depth: usize) -> String {
    let mut out = Vec::new();
    for n in nodes {
        render_node(n, depth, &mut out);
    }
    out.join("\n")
}

// ---- editing helpers -------------------------------------------------------

pub fn find<'a>(nodes: &'a [Node], name: &str) -> Option<&'a Node> {
    nodes.iter().find(|n| n.name == name)
}

pub fn get_str(nodes: &[Node], name: &str) -> Option<String> {
    find(nodes, name).and_then(|n| n.args.first()).map(|a| a.text.clone())
}

pub fn has(nodes: &[Node], name: &str) -> bool {
    find(nodes, name).is_some()
}

/// Replace the first node called `name` with `new` (or append it); `None`
/// removes it. Position is kept so a round trip doesn't reshuffle the file.
pub fn put(nodes: &mut Vec<Node>, name: &str, new: Option<Node>) {
    match (nodes.iter().position(|n| n.name == name), new) {
        (Some(i), Some(n)) => nodes[i] = n,
        (Some(i), None) => {
            nodes.remove(i);
        }
        (None, Some(n)) => nodes.push(n),
        (None, None) => {}
    }
}

/// The children of the top-level block `name`, creating it when absent.
pub fn block_mut<'a>(nodes: &'a mut Vec<Node>, name: &str) -> &'a mut Vec<Node> {
    let i = match nodes.iter().position(|n| n.name == name) {
        Some(i) => i,
        None => {
            nodes.push(Node::block(name, vec![]));
            nodes.len() - 1
        }
    };
    nodes[i].children.get_or_insert_with(Vec::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KB: &str = r#"
    keyboard {
        // install.sh detects your layout
        xkb {
            layout "gb"
            options "terminate:ctrl_alt_bksp"   // keep
        }
        repeat-delay 600
        numlock
    }
    mouse {}
"#;

    #[test]
    fn parses_nested_blocks_args_and_comments() {
        let n = parse(KB).unwrap();
        assert_eq!(n.len(), 2);
        let kb = n[0].children.as_ref().unwrap();
        let xkb = find(kb, "xkb").unwrap().children.as_ref().unwrap();
        assert_eq!(get_str(xkb, "layout").as_deref(), Some("gb"));
        assert_eq!(get_str(xkb, "options").as_deref(), Some("terminate:ctrl_alt_bksp"));
        assert_eq!(get_str(kb, "repeat-delay").as_deref(), Some("600"));
        assert!(has(kb, "numlock"));
        assert_eq!(n[1].children, Some(vec![]));
    }

    #[test]
    fn round_trips_without_losing_settings() {
        let n = parse(KB).unwrap();
        let again = parse(&render(&n, 1)).unwrap();
        assert_eq!(n, again);
        assert!(render(&n, 1).contains("    keyboard {\n        xkb {\n            layout \"gb\""));
    }

    #[test]
    fn comment_slashes_inside_strings_survive() {
        let n = parse("a \"x // y\" // real comment").unwrap();
        assert_eq!(n[0].args[0].text, "x // y");
    }

    #[test]
    fn bare_value_properties_round_trip() {
        let n = parse("scroll-factor vertical=1.0 horizontal=-1.0").unwrap();
        assert_eq!(prop(&n[0], "horizontal"), Some("-1.0"));
        assert_eq!(prop(&n[0], "vertical"), Some("1.0"));
        assert_eq!(prop(&n[0], "vert"), None);
        assert_eq!(render(&n, 0), "scroll-factor vertical=1.0 horizontal=-1.0");
    }

    #[test]
    fn refuses_syntax_it_cannot_round_trip() {
        for bad in [
            "tap; dwt",
            "scroll-factor vertical=\"2.0\"",
            "a =1",
            "a b==1",
            "/-touchpad { tap }",
            "a {\n b\n",
            "}",
            "a \"unterminated",
            "a \"esc\\\"aped\"",
        ] {
            assert!(parse(bad).is_err(), "should refuse {bad:?}");
        }
    }

    #[test]
    fn put_replaces_in_place_appends_and_removes() {
        let mut v = parse("a 1\nb 2\nc 3").unwrap();
        put(&mut v, "b", Some(Node::num("b", "9")));
        put(&mut v, "d", Some(Node::flag("d")));
        put(&mut v, "a", None);
        assert_eq!(render(&v, 0), "b 9\nc 3\nd");
        let mut v = vec![];
        block_mut(&mut v, "touchpad").push(Node::flag("tap"));
        assert_eq!(render(&v, 1), "    touchpad {\n        tap\n    }");
    }
}
