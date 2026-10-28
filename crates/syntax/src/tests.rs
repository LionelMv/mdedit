use super::*;

fn s<'a>(text: &'a str, r: &Range<usize>) -> &'a str {
    &text[r.clone()]
}

fn code(b: &Block) -> &CodeBlockInfo {
    match &b.kind {
        BlockKind::CodeBlock(i) => i,
        k => panic!("not code: {k:?}"),
    }
}

fn all_blocks(blocks: &[Block]) -> Vec<&Block> {
    let mut out = Vec::new();
    fn walk<'a>(bs: &'a [Block], out: &mut Vec<&'a Block>) {
        for b in bs {
            out.push(b);
            walk(&b.children, out);
        }
    }
    walk(blocks, &mut out);
    out
}

fn check_invariants(text: &str, blocks: &[Block]) {
    fn ok(text: &str, r: &Range<usize>) {
        assert!(r.start <= r.end && r.end <= text.len(), "bad range {r:?} len {}", text.len());
        assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end), "not boundary {r:?}");
    }
    fn walk(text: &str, bs: &[Block], parent: Option<&Range<usize>>) {
        let mut last = 0;
        for b in bs {
            ok(text, &b.range);
            ok(text, &b.content);
            assert!(b.range.start >= last, "overlap/order at {:?}", b.range);
            last = b.range.end;
            if let Some(p) = parent {
                assert!(p.start <= b.range.start && b.range.end <= p.end, "child {:?} outside {:?}", b.range, p);
            }
            assert!(b.range.start <= b.content.start && b.content.end <= b.range.end, "content outside range {b:?}");
            for sp in &b.inlines {
                ok(text, &sp.range);
                ok(text, &sp.content);
                assert!(sp.range.start <= sp.content.start && sp.content.end <= sp.range.end);
            }
            if let BlockKind::CodeBlock(info) = &b.kind {
                if let Some(r) = &info.open_fence {
                    ok(text, r);
                }
                if let Some(r) = &info.close_fence {
                    ok(text, r);
                }
            }
            if let BlockKind::Table { rows, .. } = &b.kind {
                for row in rows {
                    ok(text, &row.range);
                    for c in &row.cells {
                        ok(text, c);
                    }
                }
            }
            walk(text, &b.children, Some(&b.range));
        }
    }
    walk(text, blocks, None);
}

#[test]
fn headings_and_paragraphs() {
    let t = "# Head #\n\nSetext\n===\n\npara *x*\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(b.len(), 3);
    assert_eq!(b[0].kind, BlockKind::Heading { level: 1 });
    assert_eq!(s(t, &b[0].range), "# Head #");
    assert_eq!(s(t, &b[0].content), "Head");
    assert_eq!(s(t, &b[1].range), "Setext\n===");
    assert_eq!(s(t, &b[1].content), "Setext");
    assert_eq!(s(t, &b[2].range), "para *x*");
    assert_eq!(b[2].inlines[0].kind, SpanKind::Emphasis);
    assert_eq!(s(t, &b[2].inlines[0].content), "x");
}

#[test]
fn heading_attrs_and_empty() {
    let t = "# H {#id}\n\n##\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(s(t, &b[0].content), "H");
    assert_eq!(b[1].kind, BlockKind::Heading { level: 2 });
    assert!(b[1].content.is_empty());
}

#[test]
fn fenced_code_with_tab_and_info() {
    let t = "```rust title=\"main.rs\"\nfn main() {\n\tprintln!(\"tab\");\n}\n```\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(b.len(), 1);
    let i = code(&b[0]);
    assert_eq!(i.info, "rust title=\"main.rs\"");
    assert_eq!(i.lang.as_deref(), Some("rust"));
    assert!(i.closed);
    assert_eq!(i.fence, Some(Fence { ch: '`', len: 3, indent: 0 }));
    assert_eq!(s(t, &b[0].content), "fn main() {\n\tprintln!(\"tab\");\n}");
    assert_eq!(s(t, i.open_fence.as_ref().unwrap()), "```rust title=\"main.rs\"");
    assert_eq!(s(t, i.close_fence.as_ref().unwrap()), "```");
    assert_eq!(s(t, &b[0].range), &t[..t.len() - 1]);
    assert!(b[0].inlines.is_empty());
    let lines: Vec<&str> = code_lines(t, &b[0]).iter().map(|r| s(t, r)).collect();
    assert_eq!(lines, vec!["fn main() {", "\tprintln!(\"tab\");", "}"]);
}

#[test]
fn backticks_inside_tilde_fence() {
    let t = "~~~~\n```\nnested backtick fence is content\n```\n~~~~\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(b.len(), 1);
    let i = code(&b[0]);
    assert_eq!(i.fence.unwrap().ch, '~');
    assert_eq!(i.fence.unwrap().len, 4);
    assert!(i.closed);
    assert_eq!(s(t, &b[0].content), "```\nnested backtick fence is content\n```");
    assert_eq!(s(t, i.close_fence.as_ref().unwrap()), "~~~~");
}

#[test]
fn indented_fence() {
    let t = "  ```\n  indented fence: two spaces stripped\n    four spaces become two\n  ```\n";
    let b = parse(t);
    check_invariants(t, &b);
    let i = code(&b[0]);
    assert_eq!(i.fence.unwrap().indent, 2);
    assert!(i.closed);
    let lines: Vec<&str> = code_lines(t, &b[0]).iter().map(|r| s(t, r)).collect();
    assert_eq!(lines, vec!["indented fence: two spaces stripped", "  four spaces become two"]);
}

#[test]
fn fence_in_list_item() {
    let t = "- item\n\n  ```js\n  let x = 1;\n  \tlet y;\n  ```\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(b.len(), 1);
    assert!(matches!(b[0].kind, BlockKind::List { ordered: false, .. }));
    let item = &b[0].children[0];
    assert!(matches!(item.kind, BlockKind::ListItem { task: None, .. }));
    let cb = item.children.iter().find(|c| matches!(c.kind, BlockKind::CodeBlock(_))).unwrap();
    let i = code(cb);
    assert_eq!(i.lang.as_deref(), Some("javascript"));
    assert_eq!(i.info, "js");
    assert_eq!(i.fence.unwrap().indent, 0);
    assert!(i.closed);
    let lines: Vec<&str> = code_lines(t, cb).iter().map(|r| s(t, r)).collect();
    assert_eq!(lines, vec!["let x = 1;", "\tlet y;"]);
}

#[test]
fn fence_in_quote() {
    let t = "> ```sh\n> echo hi\n> ```\n";
    let b = parse(t);
    check_invariants(t, &b);
    let cb = &b[0].children[0];
    let i = code(cb);
    assert_eq!(i.lang.as_deref(), Some("bash"));
    assert!(i.closed);
    assert_eq!(s(t, i.close_fence.as_ref().unwrap()), "```");
    let lines: Vec<&str> = code_lines(t, cb).iter().map(|r| s(t, r)).collect();
    assert_eq!(lines, vec!["echo hi"]);
}

#[test]
fn unclosed_fence_runs_to_eof() {
    let t = "para\n\n```unclosed\nruns to end of document\n\n# not a heading\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(b.len(), 2);
    let i = code(&b[1]);
    assert!(!i.closed);
    assert!(i.close_fence.is_none());
    assert_eq!(i.lang.as_deref(), Some("unclosed"));
    assert!(s(t, &b[1].content).ends_with("# not a heading"));
    assert!(s(t, &b[1].content).starts_with("runs to end"));
}

#[test]
fn unclosed_empty_fence() {
    for t in ["```", "```\n", "```py\n\n"] {
        let b = parse(t);
        check_invariants(t, &b);
        let i = code(&b[0]);
        assert!(!i.closed, "{t:?}");
    }
    let b = parse("```");
    assert!(code_lines("```", &b[0]).is_empty());
}

#[test]
fn empty_closed_fence() {
    let t = "```\n```";
    let b = parse(t);
    let i = code(&b[0]);
    assert!(i.closed);
    assert_eq!(b[0].content, 4..4);
    assert!(code_lines(t, &b[0]).is_empty());
    let t = "```\n\n```";
    let b = parse(t);
    assert!(code(&b[0]).closed);
    assert_eq!(code_lines(t, &b[0]), vec![4..4]);
}

#[test]
fn two_blocks_stay_two() {
    let t = "```python\nprint(1)\n```\n\n```bash\necho 2\n```\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(b.len(), 2);
    assert_eq!(code(&b[0]).lang.as_deref(), Some("python"));
    assert_eq!(code(&b[1]).lang.as_deref(), Some("bash"));
    assert_eq!(s(t, &b[0].content), "print(1)");
    assert_eq!(s(t, &b[1].content), "echo 2");
}

#[test]
fn indented_code() {
    let t = "para\n\n    indented\n    \tcode\n\nafter\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(b.len(), 3);
    let i = code(&b[1]);
    assert!(i.fence.is_none());
    assert!(i.closed);
    let lines: Vec<&str> = code_lines(t, &b[1]).iter().map(|r| s(t, r)).collect();
    assert_eq!(lines, vec!["indented", "\tcode"]);
}

#[test]
fn crlf_fence() {
    let t = "```rs\r\nfn x() {}\r\n```\r\n";
    let b = parse(t);
    check_invariants(t, &b);
    let i = code(&b[0]);
    assert!(i.closed);
    assert_eq!(s(t, i.open_fence.as_ref().unwrap()), "```rs");
    assert_eq!(s(t, &b[0].content), "fn x() {}");
}

#[test]
fn closing_fence_must_be_long_enough() {
    let t = "````\n```\nstill code\n````\n";
    let b = parse(t);
    assert_eq!(b.len(), 1);
    assert_eq!(s(t, &b[0].content), "```\nstill code");
}

#[test]
fn code_line_with_trailing_backticks_is_not_a_close() {
    let t = "```\nfoo ```";
    let b = parse(t);
    assert!(!code(&b[0]).closed);
}

#[test]
fn tasks_and_lists() {
    let t = "- [ ] todo\n- [x] done\n- plain\n\n3. three\n4. four\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(b.len(), 2);
    let items = &b[0].children;
    assert_eq!(items.len(), 3);
    match &items[0].kind {
        BlockKind::ListItem { task: Some(false), checkbox: Some(cb) } => assert_eq!(s(t, cb), "[ ]"),
        k => panic!("{k:?}"),
    }
    assert!(matches!(items[1].kind, BlockKind::ListItem { task: Some(true), .. }));
    assert!(matches!(items[2].kind, BlockKind::ListItem { task: None, .. }));
    assert_eq!(s(t, &items[0].content), "todo");
    // tight item text lives in a synthetic paragraph
    assert_eq!(items[0].children.len(), 1);
    assert_eq!(items[0].children[0].kind, BlockKind::Paragraph);
    assert_eq!(s(t, &items[0].children[0].range), "todo");
    assert_eq!(b[1].kind, BlockKind::List { ordered: true, start: 3 });
    assert_eq!(s(t, &b[1].range), "3. three\n4. four");
}

#[test]
fn nested_tight_list() {
    let t = "- a **b**\n  - c\n- d\n";
    let b = parse(t);
    check_invariants(t, &b);
    let first = &b[0].children[0];
    assert_eq!(first.children.len(), 2);
    assert_eq!(s(t, &first.children[0].range), "a **b**");
    assert_eq!(first.children[0].inlines[0].kind, SpanKind::Strong);
    assert!(matches!(first.children[1].kind, BlockKind::List { .. }));
}

#[test]
fn inline_spans() {
    let t = "x **b** *e* ~~s~~ `c` [l](http://x) ![alt *t*](i.png) <http://a.b> $m$ $$d$$ [^1] <b>h</b> a\\\nb\n\n[^1]: note\n";
    let b = parse(t);
    check_invariants(t, &b);
    let p = &b[0];
    let find = |pred: &dyn Fn(&SpanKind) -> bool| p.inlines.iter().find(|s| pred(&s.kind)).unwrap().clone();
    let sp = find(&|k| *k == SpanKind::Strong);
    assert_eq!((s(t, &sp.range), s(t, &sp.content)), ("**b**", "b"));
    let sp = find(&|k| *k == SpanKind::Strikethrough);
    assert_eq!((s(t, &sp.range), s(t, &sp.content)), ("~~s~~", "s"));
    let sp = find(&|k| *k == SpanKind::Code);
    assert_eq!((s(t, &sp.range), s(t, &sp.content)), ("`c`", "c"));
    let sp = find(&|k| matches!(k, SpanKind::Link { .. }));
    assert_eq!((s(t, &sp.range), s(t, &sp.content)), ("[l](http://x)", "l"));
    assert_eq!(sp.kind, SpanKind::Link { url: "http://x".into() });
    let sp = find(&|k| matches!(k, SpanKind::Image { .. }));
    assert_eq!(s(t, &sp.content), "alt *t*");
    assert_eq!(sp.kind, SpanKind::Image { url: "i.png".into(), alt: "alt t".into() });
    let sp = find(&|k| matches!(k, SpanKind::Autolink { .. }));
    assert_eq!(s(t, &sp.content), "http://a.b");
    let sp = find(&|k| *k == SpanKind::InlineMath);
    assert_eq!(s(t, &sp.content), "m");
    let sp = find(&|k| *k == SpanKind::DisplayMath);
    assert_eq!(s(t, &sp.content), "d");
    let sp = find(&|k| matches!(k, SpanKind::FootnoteReference { .. }));
    assert_eq!(s(t, &sp.content), "1");
    assert!(p.inlines.iter().any(|s| s.kind == SpanKind::Html));
    assert!(p.inlines.iter().any(|s| s.kind == SpanKind::HardBreak));
    // pre-order
    for w in p.inlines.windows(2) {
        assert!(w[0].range.start <= w[1].range.start);
    }
    assert!(matches!(&b[1].kind, BlockKind::FootnoteDefinition { label } if label == "1"));
}

#[test]
fn code_span_space_stripping() {
    let t = "`` `x` ``";
    let b = parse(t);
    let sp = &b[0].inlines[0];
    assert_eq!(s(t, &sp.content), "`x`");
}

#[test]
fn table_cells() {
    let t = "| a | b \\| c |\n|:-|-:|\n| `x` |  |\n";
    let b = parse(t);
    check_invariants(t, &b);
    match &b[0].kind {
        BlockKind::Table { aligns, rows } => {
            assert_eq!(aligns, &vec![Align::Left, Align::Right]);
            assert_eq!(rows.len(), 2);
            assert!(rows[0].header);
            assert_eq!(s(t, &rows[0].cells[0]), "a");
            assert_eq!(s(t, &rows[0].cells[1]), "b \\| c");
            assert_eq!(s(t, &rows[1].cells[0]), "`x`");
            assert!(rows[1].cells[1].is_empty());
            assert_eq!(s(t, &rows[1].range), "| `x` |  |");
        }
        k => panic!("{k:?}"),
    }
    assert!(b[0].inlines.iter().any(|s| s.kind == SpanKind::Code));
}

#[test]
fn front_matter_only_at_start() {
    let t = "---\ntitle: x\n---\n\n# Doc\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(b[0].kind, BlockKind::FrontMatter);
    assert_eq!(s(t, &b[0].content), "title: x");
    let t2 = "text\n\n---\ntitle: x\n---\n";
    let b2 = parse(t2);
    check_invariants(t2, &b2);
    assert!(b2.iter().all(|b| b.kind != BlockKind::FrontMatter));
    assert_eq!(b2[1].kind, BlockKind::ThematicBreak);
}

#[test]
fn math_block_html_rule_quote() {
    let t = "$$\nx^2\n$$\n\n<div>\nhi\n</div>\n\n***\n\n> quote\n> more\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(b[0].kind, BlockKind::MathBlock);
    assert_eq!(s(t, &b[0].content), "x^2");
    assert_eq!(b[1].kind, BlockKind::Html);
    assert_eq!(s(t, &b[1].range), "<div>\nhi\n</div>");
    assert_eq!(b[2].kind, BlockKind::ThematicBreak);
    assert_eq!(b[3].kind, BlockKind::BlockQuote);
    assert_eq!(s(t, &b[3].range), "> quote\n> more");
}

#[test]
fn outline_and_plain_text() {
    let t = "# Title with `code` and **bold**\n\n> ## Quoted [link](http://x)\n\n- ### In list\n\ntext\n\nSub\n---\n";
    let b = parse(t);
    let o = outline(t, &b);
    let texts: Vec<(&str, u8)> = o.iter().map(|h| (h.text.as_str(), h.level)).collect();
    assert_eq!(texts, vec![("Title with code and bold", 1), ("Quoted link", 2), ("In list", 3), ("Sub", 2)]);
    assert_eq!(o[0].at, 0);
}

#[test]
fn lang_aliases() {
    for (a, b) in [
        ("rs", "rust"),
        ("PY", "python"),
        ("sh", "bash"),
        ("shell", "bash"),
        ("zsh", "bash"),
        ("console", "bash"),
        ("yml", "yaml"),
        ("js", "javascript"),
        ("jsx", "javascript"),
        ("mjs", "javascript"),
        ("ts", "typescript"),
        ("tsx", "typescript"),
        ("c++", "cpp"),
        ("cxx", "cpp"),
        ("hpp", "cpp"),
        ("cs", "csharp"),
        ("golang", "go"),
        ("md", "markdown"),
        ("kt", "kotlin"),
        ("rb", "ruby"),
        ("ps1", "powershell"),
        ("Dockerfile", "dockerfile"),
        ("make", "makefile"),
        ("Makefile", "makefile"),
        ("toml", "toml"),
        ("json", "json"),
        ("html", "html"),
        ("css", "css"),
        ("sql", "sql"),
        ("weird-lang", "weird-lang"),
    ] {
        assert_eq!(normalize_lang(a), b, "{a}");
    }
    let t = "```{.rs}\nx\n```\n```rust,ignore\ny\n```\n";
    let bl = parse(t);
    assert_eq!(code(&bl[0]).lang.as_deref(), Some("rust"));
    assert_eq!(code(&bl[1]).lang.as_deref(), Some("rust"));
}

#[test]
fn info_entity_decoding() {
    let t = "```c&#43;&#43;\nint x;\n```\n";
    let b = parse(t);
    assert_eq!(code(&b[0]).lang.as_deref(), Some("cpp"));
    assert_eq!(code(&b[0]).info, "c&#43;&#43;");
}

#[test]
fn words() {
    let t = "# Hello world\n\nSome **bold** text with `code` and [a link](http://example.com) <http://x.y>.\n\n```\nnot counted here\n```\n\n- item one — two\n";
    let b = parse(t);
    let (w, c) = word_count(t, &b);
    // Hello world Some bold text with and a link . item one two
    assert_eq!(w, 2 + 7 + 3, "words");
    assert!(c > 0);
    assert_eq!(word_count("", &[]), (0, 0));
    assert_eq!(word_count("foo**bar**", &[]).0, 1);
    assert_eq!(word_count("héllo wörld", &[]), (2, 10));
    assert_eq!(word_count("---\ntitle: x\n---\n\nbody", &[]).0, 1);
}

#[test]
fn utf8_ranges() {
    let t = "# Ünïcödé 🎉\n\n- [ ] täsk ✓\n\n| é | 🎉 |\n|---|---|\n| ü | ö |\n\n```ü\n日本語\n```\n";
    let b = parse(t);
    check_invariants(t, &b);
    assert_eq!(s(t, &b[0].content), "Ünïcödé 🎉");
}

#[test]
fn block_at_finds_innermost() {
    let t = "- a\n- b\n\n> q\n";
    let b = parse(t);
    let x = commands::block_at(&b, 6).unwrap();
    assert_eq!(x.kind, BlockKind::Paragraph);
    assert_eq!(s(t, &x.range), "b");
    let q = commands::block_at(&b, 11).unwrap();
    assert_eq!(q.kind, BlockKind::Paragraph);
    assert!(commands::block_at(&b, 100).is_none());
}

fn gen_doc(target: usize) -> String {
    let mut s = String::with_capacity(target + 1024);
    let mut i = 0;
    while s.len() < target {
        s.push_str(&format!(
            "## Section {i}\n\nSome *prose* with **bold**, `code` and a [link](https://example.com/{i}).\n\
             Second line of the paragraph.\n\n- item one\n- [ ] task two\n  - nested\n\n\
             ```rust\nfn f{i}() {{\n\tlet x = {i};\n}}\n```\n\n| a | b |\n|---|---|\n| {i} | x |\n\n> quote {i}\n\n"
        ));
        i += 1;
    }
    s
}

#[test]
fn parse_5mb_document() {
    let doc = gen_doc(5 * 1024 * 1024);
    let t0 = std::time::Instant::now();
    let b = parse(&doc);
    let dt = t0.elapsed();
    assert!(b.len() > 10_000);
    let mbps = doc.len() as f64 / 1e6 / dt.as_secs_f64().max(1e-9);
    eprintln!("parsed {} bytes in {:?} ({mbps:.1} MB/s)", doc.len(), dt);
    if !cfg!(debug_assertions) {
        assert!(mbps > 8.0, "parse too slow: {mbps:.1} MB/s");
    }
    // top-level order
    for w in b.windows(2) {
        assert!(w[0].range.end <= w[1].range.start);
    }
}

mod props {
    use super::*;
    use proptest::prelude::*;

    fn md_piece() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("```".to_string()),
            Just("~~~~".to_string()),
            Just("```rust x=1".to_string()),
            Just("\t".to_string()),
            Just("  ".to_string()),
            Just("\n".to_string()),
            Just("\r\n".to_string()),
            Just("> ".to_string()),
            Just("- ".to_string()),
            Just("1. ".to_string()),
            Just("[ ] ".to_string()),
            Just("[x]".to_string()),
            Just("# ".to_string()),
            Just("**".to_string()),
            Just("*".to_string()),
            Just("_".to_string()),
            Just("`".to_string()),
            Just("|".to_string()),
            Just("|---|".to_string()),
            Just("$".to_string()),
            Just("$$".to_string()),
            Just("[^1]".to_string()),
            Just("[^1]: ".to_string()),
            Just("](http://x)".to_string()),
            Just("![".to_string()),
            Just("[".to_string()),
            Just("<div>".to_string()),
            Just("<".to_string()),
            Just("---".to_string()),
            Just("\\".to_string()),
            Just("&amp;".to_string()),
            "[a-zé🎉 ]{0,6}",
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]
        #[test]
        fn parse_never_panics_and_ranges_valid(pieces in proptest::collection::vec(md_piece(), 0..40)) {
            let t: String = pieces.concat();
            let b = parse(&t);
            check_invariants(&t, &b);
            let _ = outline(&t, &b);
            let _ = word_count(&t, &b);
            for blk in all_blocks(&b) {
                let _ = plain_text(&t, blk);
                for r in code_lines(&t, blk) {
                    prop_assert!(r.start <= r.end && r.end <= t.len());
                    prop_assert!(t.is_char_boundary(r.start) && t.is_char_boundary(r.end));
                }
            }
        }

        #[test]
        fn arbitrary_strings(t in "\\PC{0,200}") {
            let b = parse(&t);
            check_invariants(&t, &b);
        }
    }
}

fn max_depth(blocks: &[Block]) -> usize {
    // iterative, so the test itself can't overflow
    let mut best = 0;
    let mut todo: Vec<(&Block, usize)> = blocks.iter().map(|b| (b, 1)).collect();
    while let Some((b, d)) = todo.pop() {
        best = best.max(d);
        todo.extend(b.children.iter().map(|c| (c, d + 1)));
    }
    best
}

fn deep_inputs() -> Vec<String> {
    vec![
        format!("{} x", ">".repeat(10_000)),
        format!("{}x", "> - ".repeat(5_000)),
        "> - ".repeat(20_000),
        ">".repeat(100_000),
        format!("{}x", "- ".repeat(5_000)),
        format!("{}x\n", "[^a]: ".repeat(2_000)),
        (0..3000).map(|i| format!("{}- item\n", "  ".repeat(i))).collect(),
    ]
}

#[test]
fn deep_nesting_is_capped_and_fast() {
    for t in deep_inputs() {
        let t0 = std::time::Instant::now();
        let b = parse(&t);
        let dt = t0.elapsed();
        assert!(max_depth(&b) <= MAX_NEST + 3, "depth {}", max_depth(&b));
        check_invariants(&t, &b);
        if !cfg!(debug_assertions) {
            assert!(dt.as_millis() < 500, "{} bytes took {dt:?}", t.len());
        }
    }
}

#[test]
fn deep_trees_clone_drop_shift_on_small_stack() {
    let h = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(|| {
            for t in deep_inputs() {
                let b = parse(&t);
                let mut c = b.clone();
                assert!(c == b);
                for blk in &mut c {
                    incremental::shift(blk, 7);
                }
                let _ = outline(&t, &b);
                drop(c);
                drop(b);
            }
        })
        .expect("spawn");
    h.join().expect("deep tree handling overflowed the stack");
}
