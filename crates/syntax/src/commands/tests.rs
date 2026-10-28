use super::*;
use crate::parse;

/// Parse a string where `|` marks the caret, or `[`..`]` mark anchor..head (written as
/// `«` and `»`), run `f`, and render the result with the same markers.
fn mk(src: &str) -> (String, Sel) {
    let mut t = String::new();
    let mut a = None;
    let mut h = None;
    for ch in src.chars() {
        match ch {
            '|' | '‸' if !src.contains('‸') || ch == '‸' => {
                a = Some(t.len());
                h = Some(t.len());
            }
            '«' => a = Some(t.len()),
            '»' => h = Some(t.len()),
            c => t.push(c),
        }
    }
    let a = a.expect("no caret");
    (t, Sel { anchor: a, head: h.unwrap_or(a) })
}

thread_local! {
    static CARET: std::cell::Cell<char> = const { std::cell::Cell::new('|') };
}

fn show(text: &str, sel: Sel) -> String {
    CARET.with(|c| c.set(if text.contains('|') { '‸' } else { '|' }));
    let mut out = String::new();
    for (i, ch) in text.char_indices() {
        push_marks(&mut out, i, sel);
        out.push(ch);
    }
    push_marks(&mut out, text.len(), sel);
    out
}

fn push_marks(out: &mut String, i: usize, sel: Sel) {
    if sel.anchor == sel.head {
        if i == sel.anchor {
            out.push(CARET.with(|c| c.get()));
        }
    } else {
        if i == sel.anchor {
            out.push('«');
        }
        if i == sel.head {
            out.push('»');
        }
    }
}

fn check_result(text: &str, r: &CommandResult) -> String {
    let mut last = 0;
    for e in &r.edits {
        assert!(e.range.start >= last && e.range.start <= e.range.end && e.range.end <= text.len());
        assert!(text.is_char_boundary(e.range.start) && text.is_char_boundary(e.range.end));
        last = e.range.end;
    }
    let out = apply(text, r);
    assert!(r.selection.anchor <= out.len() && r.selection.head <= out.len());
    assert!(out.is_char_boundary(r.selection.anchor) && out.is_char_boundary(r.selection.head));
    out
}

fn cmd(src: &str, c: Command) -> Option<String> {
    let (t, sel) = mk(src);
    let b = parse(&t);
    run(&t, &b, sel, &c).map(|r| {
        let out = check_result(&t, &r);
        show(&out, r.selection)
    })
}

fn key(src: &str, f: impl Fn(&str, &[Block], Sel) -> Option<CommandResult>) -> Option<String> {
    let (t, sel) = mk(src);
    let b = parse(&t);
    f(&t, &b, sel).map(|r| {
        let out = check_result(&t, &r);
        show(&out, r.selection)
    })
}

fn enter(src: &str) -> Option<String> {
    key(src, on_enter)
}
fn tab(src: &str) -> Option<String> {
    key(src, |t, b, s| on_tab(t, b, s, false))
}
fn stab(src: &str) -> Option<String> {
    key(src, |t, b, s| on_tab(t, b, s, true))
}
fn bs(src: &str) -> Option<String> {
    key(src, on_backspace)
}
fn ch(src: &str, c: char) -> Option<String> {
    key(src, |t, b, s| on_char(t, b, s, c))
}

// ----- inline toggles -----

#[test]
fn bold_wraps_selection() {
    assert_eq!(cmd("a «word» b", Command::Bold).unwrap(), "a **«word»** b");
}

#[test]
fn bold_empty_inserts_pair() {
    assert_eq!(cmd("a | b", Command::Bold).unwrap(), "a **|** b");
}

#[test]
fn bold_toggle_off_caret_inside() {
    assert_eq!(cmd("a **wo|rd** b", Command::Bold).unwrap(), "a wo|rd b");
}

#[test]
fn bold_toggle_off_selection_of_content() {
    assert_eq!(cmd("a **«word»** b", Command::Bold).unwrap(), "a «word» b");
}

#[test]
fn bold_toggle_off_whole_span_selected() {
    assert_eq!(cmd("a «**word**» b", Command::Bold).unwrap(), "a «word» b");
}

#[test]
fn italic_inside_bold_wraps() {
    assert_eq!(cmd("**a «b» c**", Command::Italic).unwrap(), "**a *«b»* c**");
}

#[test]
fn italic_toggle_off() {
    assert_eq!(cmd("x *it|al* y", Command::Italic).unwrap(), "x it|al y");
}

#[test]
fn strike_and_code() {
    assert_eq!(cmd("«x»", Command::Strikethrough).unwrap(), "~~«x»~~");
    assert_eq!(cmd("«x»", Command::InlineCode).unwrap(), "`«x»`");
    assert_eq!(cmd("a `c|ode` b", Command::InlineCode).unwrap(), "a c|ode b");
    // backticks inside the selection get a longer fence and padding
    assert_eq!(cmd("«a`b»", Command::InlineCode).unwrap(), "``«a`b»``");
    assert_eq!(cmd("«`x» y", Command::InlineCode).unwrap(), "`` «`x» `` y");
    // selecting an existing code span removes its backticks
    assert_eq!(cmd("«`x`»", Command::InlineCode).unwrap(), "«x»");
}

#[test]
fn reversed_selection_keeps_direction() {
    assert_eq!(cmd("a »word« b", Command::Bold).unwrap(), "a **»word«** b");
}

#[test]
fn inline_commands_ignored_in_code() {
    assert!(cmd("```\nco|de\n```", Command::Bold).is_none());
}

// ----- links -----

#[test]
fn link_commands() {
    assert_eq!(cmd("see «docs» now", Command::Link).unwrap(), "see [docs](«url») now");
    assert_eq!(cmd("«https://x.org»", Command::Link).unwrap(), "[|](https://x.org)");
    assert_eq!(cmd("a | b", Command::Link).unwrap(), "a [|](url) b");
    assert_eq!(cmd("a [te|xt](http://x) b", Command::Link).unwrap(), "a «text» b");
    assert_eq!(cmd("«pic»", Command::Image).unwrap(), "![pic](«url»)");
    assert_eq!(cmd("|", Command::Image).unwrap(), "![|](url)");
}

// ----- headings -----

#[test]
fn heading_set_change_remove() {
    assert_eq!(cmd("hel|lo", Command::Heading(2)).unwrap(), "## hel|lo");
    assert_eq!(cmd("# hel|lo", Command::Heading(3)).unwrap(), "### hel|lo");
    assert_eq!(cmd("## hel|lo", Command::Heading(2)).unwrap(), "hel|lo");
    assert_eq!(cmd("### hel|lo", Command::Heading(0)).unwrap(), "hel|lo");
    assert!(cmd("hel|lo", Command::Heading(0)).is_none());
    assert_eq!(cmd("> quo|te", Command::Heading(1)).unwrap(), "> # quo|te");
    assert_eq!(cmd("|# x", Command::Heading(2)).unwrap(), "## |x");
}

#[test]
fn heading_setext() {
    assert_eq!(cmd("Ti|tle\n===\n\nnext", Command::Heading(2)).unwrap(), "## Ti|tle\n\nnext");
    assert_eq!(cmd("Ti|tle\n---", Command::Heading(2)).unwrap(), "Ti|tle");
}

// ----- line prefixes -----

#[test]
fn bullet_list_toggle() {
    assert_eq!(cmd("«a\nb»", Command::BulletList).unwrap(), "«- a\n- b»");
    assert_eq!(cmd("«- a\n- b»", Command::BulletList).unwrap(), "«a\nb»");
    assert_eq!(cmd("it|em", Command::BulletList).unwrap(), "- it|em");
    assert_eq!(cmd("1. it|em", Command::BulletList).unwrap(), "- it|em");
    assert_eq!(cmd("«a\n\nb»", Command::BulletList).unwrap(), "«- a\n\n- b»");
}

#[test]
fn ordered_list_numbers() {
    assert_eq!(cmd("«a\nb\nc»", Command::OrderedList).unwrap(), "«1. a\n2. b\n3. c»");
    assert_eq!(cmd("«1. a\n2. b»", Command::OrderedList).unwrap(), "«a\nb»");
    assert_eq!(cmd("- x|", Command::OrderedList).unwrap(), "1. x|");
}

#[test]
fn task_list_toggle() {
    assert_eq!(cmd("x|", Command::TaskList).unwrap(), "- [ ] x|");
    assert_eq!(cmd("- x|", Command::TaskList).unwrap(), "- [ ] x|");
    assert_eq!(cmd("- [x] x|", Command::TaskList).unwrap(), "x|");
    assert_eq!(cmd("  * y|", Command::TaskList).unwrap(), "  * [ ] y|");
}

#[test]
fn quote_toggle() {
    assert_eq!(cmd("«a\nb»", Command::BlockQuote).unwrap(), "«> a\n> b»");
    assert_eq!(cmd("«> a\n> b»", Command::BlockQuote).unwrap(), "«a\nb»");
    assert_eq!(cmd("> > x|", Command::BlockQuote).unwrap(), "> x|");
}

#[test]
fn selection_ending_at_line_start_excludes_line() {
    assert_eq!(cmd("«a\n»b", Command::BulletList).unwrap(), "«- a\n»b");
}

// ----- code / math blocks -----

#[test]
fn code_block_on_empty_line() {
    assert_eq!(cmd("para\n\n|", Command::CodeBlock).unwrap(), "para\n\n```\n|\n```");
}

#[test]
fn code_block_after_text_line() {
    assert_eq!(cmd("te|xt\nmore", Command::CodeBlock).unwrap(), "text\n```\n|\n```\nmore");
}

#[test]
fn code_block_wraps_lines_with_long_fence() {
    assert_eq!(cmd("«a ```x``` b\nc»", Command::CodeBlock).unwrap(), "````\n«a ```x``` b\nc»\n````");
    assert_eq!(cmd("x\n«mid»dle\ny", Command::CodeBlock).unwrap(), "x\n```\n«middle»\n```\ny");
}

#[test]
fn code_block_toggle_off() {
    assert_eq!(cmd("```rs\nco|de\n```\n", Command::CodeBlock).unwrap(), "co|de\n");
}

#[test]
fn math_block() {
    assert_eq!(cmd("|", Command::MathBlock).unwrap(), "$$\n|\n$$");
    assert_eq!(cmd("«x^2»", Command::MathBlock).unwrap(), "$$\n«x^2»\n$$");
}

#[test]
fn fence_for_rule() {
    assert_eq!(fence_for("no ticks", '`'), "```");
    assert_eq!(fence_for("````", '`'), "`````");
    assert_eq!(fence_for("~~~", '~'), "~~~~");
}

#[test]
fn hr_and_table() {
    assert_eq!(cmd("text|", Command::HorizontalRule).unwrap(), "text\n\n---\n|");
    assert_eq!(cmd("|", Command::HorizontalRule).unwrap(), "---|");
    assert_eq!(cmd("a\n|", Command::HorizontalRule).unwrap(), "a\n\n---|");
    assert_eq!(
        cmd("|", Command::Table { cols: 2, rows: 1 }).unwrap(),
        "| «Column 1» | Column 2 |\n| --- | --- |\n|  |  |"
    );
    let t = cmd("para|", Command::Table { cols: 1, rows: 0 }).unwrap();
    assert_eq!(t, "para\n\n| «Column 1» |\n| --- |");
    // the skeleton parses as a table
    let raw = t.replace(['«', '»'], "");
    assert!(matches!(parse(&raw)[1].kind, BlockKind::Table { .. }));
}

#[test]
fn toggle_task() {
    assert_eq!(cmd("- [ ] do |it", Command::ToggleTask).unwrap(), "- [x] do |it");
    assert_eq!(cmd("- [x] do |it", Command::ToggleTask).unwrap(), "- [ ] do |it");
    assert_eq!(cmd("- do |it", Command::ToggleTask).unwrap(), "- [ ] do |it");
    assert!(cmd("not a |list", Command::ToggleTask).is_none());
    assert_eq!(cmd("- a\n  - [ ] n|ested", Command::ToggleTask).unwrap(), "- a\n  - [x] n|ested");
}

// ----- Enter -----

#[test]
fn enter_continues_bullets() {
    assert_eq!(enter("- one|").unwrap(), "- one\n- |");
    assert_eq!(enter("* one|").unwrap(), "* one\n* |");
    assert_eq!(enter("  - nested|").unwrap(), "  - nested\n  - |");
    assert_eq!(enter("- sp|lit").unwrap(), "- sp\n- |lit");
}

#[test]
fn enter_ordered_increments() {
    assert_eq!(enter("1. one|").unwrap(), "1. one\n2. |");
    assert_eq!(enter("9) nine|").unwrap(), "9) nine\n10) |");
}

#[test]
fn enter_task_items() {
    assert_eq!(enter("- [x] done|").unwrap(), "- [x] done\n- [ ] |");
    assert_eq!(enter("- [ ] todo|").unwrap(), "- [ ] todo\n- [ ] |");
}

#[test]
fn enter_on_empty_item_ends_list() {
    assert_eq!(enter("- one\n- |").unwrap(), "- one\n|");
    assert_eq!(enter("1. one\n2. |").unwrap(), "1. one\n|");
    assert_eq!(enter("- [ ] a\n- [ ] |").unwrap(), "- [ ] a\n|");
}

#[test]
fn enter_quote() {
    assert_eq!(enter("> quote|").unwrap(), "> quote\n> |");
    assert_eq!(enter("> > deep|").unwrap(), "> > deep\n> > |");
    assert_eq!(enter("> a\n> |").unwrap(), "> a\n|");
    assert_eq!(enter("> - item|").unwrap(), "> - item\n> - |");
}

#[test]
fn enter_in_code_autoindents() {
    assert_eq!(enter("```\n    indented|\n```").unwrap(), "```\n    indented\n    |\n```");
    assert_eq!(enter("```\n\tx|\n```").unwrap(), "```\n\tx\n\t|\n```");
    // no list continuation inside code
    assert_eq!(enter("```\n- item|\n```").unwrap(), "```\n- item\n|\n```");
    assert_eq!(enter("> ```\n> code|\n> ```").unwrap(), "> ```\n> code\n> |\n> ```");
}

#[test]
fn enter_completes_fence() {
    assert_eq!(enter("```rust|").unwrap(), "```rust\n|\n```");
    assert_eq!(enter("para\n\n~~~~|").unwrap(), "para\n\n~~~~\n|\n~~~~");
    assert_eq!(enter("- item\n\n  ```py|").unwrap(), "- item\n\n  ```py\n  |\n  ```");
    // trailing text after the fence that swallows the rest of the doc
    assert_eq!(enter("```|\n\nlater text").unwrap(), "```\n|\n```\n\nlater text");
    // closed block: plain newline
    assert!(enter("```rust|\ncode\n```").is_none());
    // caret in the middle of the info string: plain newline
    assert!(enter("```ru|st").is_none());
}

#[test]
fn enter_plain_paragraph_is_none() {
    assert!(enter("hello|").is_none());
    assert!(enter("# heading|").is_none());
    assert!(enter("«sel»").is_none());
}

// ----- Tab -----

#[test]
fn tab_in_code_indents() {
    assert_eq!(tab("```\nx|\n```").unwrap(), "```\nx    |\n```");
    assert_eq!(tab("```\n\ta\n|b\n```").unwrap(), "```\n\ta\n\t|b\n```");
    assert_eq!(tab("```\n«a\nb»\n```").unwrap(), "```\n    «a\n    b»\n```");
    assert_eq!(stab("```\n    «a\n\tb»\n```").unwrap(), "```\n«a\nb»\n```");
    assert!(tab("```|\nx\n```").is_none());
}

#[test]
fn tab_in_list() {
    assert_eq!(tab("- a\n- b|").unwrap(), "- a\n  - b|");
    assert_eq!(stab("- a\n  - b|").unwrap(), "- a\n- b|");
    assert_eq!(tab("1. a\n2. b|").unwrap(), "1. a\n   2. b|");
    assert_eq!(stab("- a|").unwrap(), "- a|"); // nothing to outdent, key consumed
}

#[test]
fn tab_in_table() {
    assert_eq!(tab("| a‸ | b |\n|---|---|\n| c | d |").unwrap(), "| a | «b» |\n|---|---|\n| c | d |");
    assert_eq!(tab("| a | b‸ |\n|---|---|\n| c | d |").unwrap(), "| a | b |\n|---|---|\n| «c» | d |");
    assert_eq!(stab("| a | b |\n|---|---|\n| c‸ | d |").unwrap(), "| a | «b» |\n|---|---|\n| c | d |");
    assert_eq!(stab("| a‸ | b |\n|---|---|").unwrap(), "| a‸ | b |\n|---|---|");
    // Tab in the last cell appends a row
    assert_eq!(tab("| a | b |\n|---|---|\n| c | d‸ |").unwrap(), "| a | b |\n|---|---|\n| c | d |\n| ‸ |  |");
    // empty cells: caret lands inside the cell
    assert_eq!(tab("| a‸ |   |\n|---|---|").unwrap(), "| a | ‸  |\n|---|---|");
}

#[test]
fn tab_plain_is_none() {
    assert!(tab("hello|").is_none());
}

// ----- Backspace -----

#[test]
fn backspace_empty_code_block() {
    assert_eq!(bs("para\n\n```\n|\n```\n").unwrap(), "para\n\n|\n");
    assert!(bs("```\nx|\n```").is_none());
    assert!(bs("```\n|x\n```").is_none());
}

#[test]
fn backspace_markers() {
    assert_eq!(bs("## |head").unwrap(), "|head");
    assert!(bs("## h|ead").is_none());
    assert_eq!(bs("- |item").unwrap(), "|item");
    assert_eq!(bs("1. |item").unwrap(), "|item");
    assert_eq!(bs("- [ ] |task").unwrap(), "- |task");
    assert_eq!(bs("> |quote").unwrap(), "|quote");
    assert_eq!(bs("> > |deep").unwrap(), "> |deep");
    assert!(bs("plain |text").is_none());
    assert!(bs("«sel»").is_none());
}

// ----- on_char -----

#[test]
fn char_wraps_selection() {
    assert_eq!(ch("«x»", '(').unwrap(), "(«x»)");
    assert_eq!(ch("«x»", '*').unwrap(), "*«x»*");
    assert_eq!(ch("«x»", '`').unwrap(), "`«x»`");
    assert_eq!(ch("«x»", '"').unwrap(), "\"«x»\"");
    assert!(ch("«x»", ')').is_none());
}

#[test]
fn char_autoclose() {
    assert_eq!(ch("a |", '(').unwrap(), "a (|)");
    assert_eq!(ch("|", '[').unwrap(), "[|]");
    assert_eq!(ch("| b", '{').unwrap(), "{|} b");
    assert!(ch("|word", '(').is_none());
    assert!(ch("a |", '*').is_none());
    assert!(ch("a |", '_').is_none());
    assert_eq!(ch("a |", '`').unwrap(), "a `|`");
    assert_eq!(ch("a |", '"').unwrap(), "a \"|\"");
    assert!(ch("a|", '"').is_none());
}

#[test]
fn char_typeover() {
    assert_eq!(ch("(a|)", ')').unwrap(), "(a)|");
    assert_eq!(ch("`a|`", '`').unwrap(), "`a`|");
    assert!(ch("a|", ')').is_none());
}

#[test]
fn typing_a_fence_gives_three_backticks() {
    let mut t = String::new();
    let mut caret = 0;
    for _ in 0..3 {
        let b = parse(&t);
        match on_char(&t, &b, Sel::caret(caret), '`') {
            Some(r) => {
                t = apply(&t, &r);
                caret = r.selection.head;
            }
            None => {
                t.insert(caret, '`');
                caret += 1;
            }
        }
    }
    assert_eq!(t, "```");
    assert_eq!(caret, 3);
}

#[test]
fn char_in_code() {
    assert!(ch("```\n«x»\n```", '*').is_none());
    assert!(ch("```\nx |\n```", '`').is_none());
    assert_eq!(ch("```\nf|\n```", '(').unwrap(), "```\nf(|)\n```");
}

// ----- set_code_lang / block_at -----

#[test]
fn set_lang() {
    let t = "```rs title=x\ncode\n```";
    let b = parse(t);
    let r = set_code_lang(t, &b, 15, "python").unwrap();
    assert_eq!(apply(t, &r), "```python title=x\ncode\n```");
    let t = "```\ncode\n```";
    let b = parse(t);
    let r = set_code_lang(t, &b, 5, "go").unwrap();
    assert_eq!(apply(t, &r), "```go\ncode\n```");
    let t = "~~~ rust x\ncode\n~~~";
    let b = parse(t);
    let r = set_code_lang(t, &b, 12, "").unwrap();
    assert_eq!(apply(t, &r), "~~~ x\ncode\n~~~");
    assert!(set_code_lang(t, &b, 12, "a b").is_none());
    let t = "```\ncode\n```";
    let b = parse(t);
    assert!(set_code_lang(t, &b, 5, "a`b").is_none());
    assert!(set_code_lang("para", &parse("para"), 1, "rust").is_none());
    // tabs in content untouched
    let t = "```make\nall:\n\tcc main.c\n```\n";
    let b = parse(t);
    let r = set_code_lang(t, &b, 10, "makefile").unwrap();
    assert_eq!(apply(t, &r), "```makefile\nall:\n\tcc main.c\n```\n");
}

#[test]
fn block_at_and_path() {
    let t = "> - a\n>   ```\n>   x\n>   ```\n";
    let b = parse(t);
    let x = t.find("x").unwrap();
    let p = block_path(&b, x);
    let kinds: Vec<&str> = p
        .iter()
        .map(|b| match b.kind {
            BlockKind::BlockQuote => "quote",
            BlockKind::List { .. } => "list",
            BlockKind::ListItem { .. } => "item",
            BlockKind::CodeBlock(_) => "code",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["quote", "list", "item", "code"]);
    assert!(matches!(block_at(&b, x).unwrap().kind, BlockKind::CodeBlock(_)));
    assert!(block_at(&[], 0).is_none());
}

// ----- robustness -----

mod props {
    use super::*;
    use proptest::prelude::*;

    fn piece() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("```".to_string()),
            Just("~~~".to_string()),
            Just("\n".to_string()),
            Just("\r\n".to_string()),
            Just("\t".to_string()),
            Just("  ".to_string()),
            Just("> ".to_string()),
            Just("- ".to_string()),
            Just("1. ".to_string()),
            Just("- [ ] ".to_string()),
            Just("# ".to_string()),
            Just("**".to_string()),
            Just("*".to_string()),
            Just("`".to_string()),
            Just("|".to_string()),
            Just("|---|---|".to_string()),
            Just("$$".to_string()),
            Just("[a](b)".to_string()),
            "[a-zé🎉 ]{0,5}",
        ]
    }

    fn all_cmds() -> Vec<Command> {
        vec![
            Command::Bold,
            Command::Italic,
            Command::Strikethrough,
            Command::InlineCode,
            Command::Link,
            Command::Image,
            Command::Heading(0),
            Command::Heading(2),
            Command::BlockQuote,
            Command::BulletList,
            Command::OrderedList,
            Command::TaskList,
            Command::CodeBlock,
            Command::MathBlock,
            Command::HorizontalRule,
            Command::Table { cols: 2, rows: 2 },
            Command::ToggleTask,
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]
        #[test]
        fn commands_never_panic(pieces in proptest::collection::vec(piece(), 0..25), a in 0usize..200, h in 0usize..200) {
            let t: String = pieces.concat();
            let b = parse(&t);
            let sel = Sel { anchor: a, head: h };
            for c in all_cmds() {
                if let Some(r) = run(&t, &b, sel, &c) { check_result(&t, &r); }
            }
            let caret = Sel::caret(a);
            for r in [
                on_enter(&t, &b, caret),
                on_enter(&t, &b, sel),
                on_tab(&t, &b, sel, false),
                on_tab(&t, &b, caret, true),
                on_backspace(&t, &b, caret),
                on_char(&t, &b, sel, '('),
                on_char(&t, &b, caret, '`'),
                on_char(&t, &b, caret, ')'),
                on_char(&t, &b, sel, '*'),
                set_code_lang(&t, &b, a, "rust"),
                set_code_lang(&t, &b, a, ""),
            ].into_iter().flatten() {
                check_result(&t, &r);
            }
        }
    }
}

// ----- integrator follow-ups -----

#[test]
fn enter_between_brackets_in_code() {
    assert_eq!(enter("```rust\nfn main() {|}\n```").unwrap(), "```rust\nfn main() {\n    |\n}\n```");
    assert_eq!(enter("```\n    f(|)\n```").unwrap(), "```\n    f(\n        |\n    )\n```");
    assert_eq!(enter("```go\nfunc x() {\n\ty := [|]\n}\n```").unwrap(), "```go\nfunc x() {\n\ty := [\n\t\t|\n\t]\n}\n```");
    // not between a pair: plain auto-indent
    assert_eq!(enter("```\n  a {|\n```").unwrap(), "```\n  a {\n  |\n```");
}

#[test]
fn enter_twice_exits_code_block() {
    assert_eq!(enter("```\ncode\n\n|\n```").unwrap(), "```\ncode\n```\n|");
    assert_eq!(enter("```\ncode\n\n|\n```\nnext").unwrap(), "```\ncode\n```\n|\nnext");
    assert_eq!(enter("```\ncode\n\n|\n```\n\nnext").unwrap(), "```\ncode\n```\n|\nnext"); // reuses the blank line after the fence
    // only one empty line: just a newline (auto-indent)
    assert_eq!(enter("```\ncode\n|\n```").unwrap(), "```\ncode\n\n|\n```");
    // empty line in the middle: no exit
    assert_eq!(enter("```\na\n\n|\nb\n```").unwrap(), "```\na\n\n\n|\nb\n```");
    // the whole block blank: no exit
    assert!(enter("```\n\n|\n```").is_some_and(|r| r.contains("```\n\n\n|")));
    assert_eq!(enter("> ```\n> x\n>\n> |\n> ```").unwrap(), "> ```\n> x\n> ```\n>|");
}

#[test]
fn enter_renumbers_ordered_list() {
    assert_eq!(enter("1. a|\n2. b\n3. c").unwrap(), "1. a\n2. |\n3. b\n4. c");
    assert_eq!(enter("1. a\n2. b|\n3. c").unwrap(), "1. a\n2. b\n3. |\n4. c");
    // not sequential: left alone
    assert_eq!(enter("1. a|\n5. b\n9. c").unwrap(), "1. a\n2. |\n5. b\n9. c");
    // nested list numbering untouched
    assert_eq!(enter("1. a|\n   1. x\n2. b").unwrap(), "1. a\n2. |\n   1. x\n3. b");
}
