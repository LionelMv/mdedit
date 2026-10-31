use super::*;
use mdedit_syntax::{CodeBlockInfo, Fence, TableRow};
use proptest::prelude::*;

fn br(sec: &egui::text::LayoutSection) -> Range<usize> {
    sec.byte_range.start.0..sec.byte_range.end.0
}

fn blk(kind: BlockKind, range: Range<usize>, content: Range<usize>) -> Block {
    Block { kind, range, content, children: Vec::new(), inlines: Vec::new() }
}

fn span(kind: SpanKind, range: Range<usize>, content: Range<usize>) -> Span {
    Span { kind, range, content }
}

fn hl() -> &'static Highlighter {
    static H: std::sync::OnceLock<Highlighter> = std::sync::OnceLock::new();
    H.get_or_init(Highlighter::synchronous)
}

fn style(text: &str, blocks: &[Block], chunk: &Chunk, mode: Mode, focused: bool, theme: &Theme) -> StyledChunk {
    style_chunk(&StyleInput {
        text,
        blocks,
        chunk,
        mode,
        focused,
        theme,
        highlighter: hl(),
        wrap_width: 600.0,
    })
}

fn deco_ranges(d: &Decoration) -> Vec<Range<usize>> {
    match d {
        Decoration::Card { range, .. }
        | Decoration::InlineBg { range, .. }
        | Decoration::QuoteBar { range, .. }
        | Decoration::Rule { range, .. }
        | Decoration::Checkbox { range, .. }
        | Decoration::Bullet { range, .. }
        | Decoration::ListNumber { range, .. }
        | Decoration::Image { range, .. }
        | Decoration::Link { range, .. } => vec![range.clone()],
        Decoration::CodeHeader { range, code, .. } => vec![range.clone(), code.clone()],
        Decoration::TableGrid { rows, .. } => rows.iter().flatten().cloned().collect(),
    }
}

/// Assert the whole chunk contract for `text` + `blocks`; returns the chunks.
fn check_contract(text: &str, blocks: &[Block]) -> Vec<Chunk> {
    let cs = chunks(text, blocks);
    assert!(!cs.is_empty());
    assert_eq!(cs[0].range.start, 0);
    assert_eq!(cs.last().map(|c| c.range.end), Some(text.len()));
    for w in cs.windows(2) {
        assert_eq!(w[0].range.end, w[1].range.start, "chunks must touch");
        assert!(w[0].range.start < w[0].range.end, "empty chunk in the middle");
        assert!(text[w[0].range.clone()].ends_with('\n'), "non-final chunk must end with \\n");
    }
    for c in &cs {
        assert!(text.is_char_boundary(c.range.start) && text.is_char_boundary(c.range.end));
        let src = &text[c.range.clone()];
        let body = src.strip_suffix('\n').unwrap_or(src);
        for theme in [Theme::light(), Theme::dark()] {
            for mode in [Mode::LivePreview, Mode::Source] {
                for focused in [false, true] {
                    let s = style(text, blocks, c, mode, focused, &theme);
                    assert_eq!(s.job.text, body, "job text must equal chunk minus final \\n");
                    // Sections cover the text contiguously, on char boundaries.
                    let mut pos = 0;
                    for sec in &s.job.sections {
                        let br = br(sec);
                        assert_eq!(br.start, pos);
                        assert!(body.is_char_boundary(br.end));
                        pos = br.end;
                        let f = &sec.format;
                        if f.font_id.size == HIDDEN_SIZE {
                            assert_eq!(f.color, Color32::TRANSPARENT);
                        }
                        if mode == Mode::Source {
                            assert!(
                                f.font_id.size == HIDDEN_SIZE || f.font_id.family == FontFamily::Monospace
                            );
                            let t = &body[br.clone()];
                            if f.font_id.size == HIDDEN_SIZE {
                                assert!(t.chars().all(|c| c == '\r'), "source mode hides only CR: {t:?}");
                            }
                        }
                    }
                    assert_eq!(pos, body.len());
                    if mode == Mode::Source {
                        assert!(s.decorations.is_empty());
                    }
                    for d in &s.decorations {
                        for r in deco_ranges(d) {
                            assert!(r.start <= r.end && r.end <= body.len(), "{d:?} out of chunk");
                            assert!(body.is_char_boundary(r.start) && body.is_char_boundary(r.end));
                        }
                    }
                }
            }
        }
    }
    cs
}

/// Concatenated job text of sections matching `pred`.
fn text_where(s: &StyledChunk, pred: impl Fn(&TextFormat) -> bool) -> String {
    s.job
        .sections
        .iter()
        .filter(|sec| pred(&sec.format))
        .map(|sec| &s.job.text[br(sec)])
        .collect()
}

fn hidden(f: &TextFormat) -> bool {
    f.font_id.size == HIDDEN_SIZE
}

fn visible_text(s: &StyledChunk) -> String {
    text_where(s, |f| !hidden(f) && f.color != Color32::TRANSPARENT)
}

// ---------------------------------------------------------------------------------------------
// chunks()

#[test]
fn empty_doc() {
    assert_eq!(chunks("", &[]), vec![Chunk { range: 0..0, block: None }]);
    check_contract("", &[]);
}

#[test]
fn chunks_follow_blocks() {
    let text = "\n\n# Title\n\npara one\nline two\n\n\n- a\n- b";
    let blocks = vec![
        blk(BlockKind::Heading { level: 1 }, 2..9, 4..9),
        blk(BlockKind::Paragraph, 11..28, 11..28),
        blk(BlockKind::List { ordered: false, start: 1 }, 31..38, 31..38),
    ];
    let cs = check_contract(text, &blocks);
    assert_eq!(
        cs,
        vec![
            Chunk { range: 0..2, block: None },
            Chunk { range: 2..11, block: Some(0) },
            Chunk { range: 11..31, block: Some(1) },
            Chunk { range: 31..38, block: Some(2) },
        ]
    );
}

#[test]
fn chunks_without_blocks_split_on_blank_lines() {
    let text = "a\nb\n\nc\n\n\nd";
    let cs = check_contract(text, &[]);
    let r: Vec<_> = cs.iter().map(|c| c.range.clone()).collect();
    assert_eq!(r, vec![0..5, 5..9, 9..10]);
}

#[test]
fn chunks_tolerate_garbage_blocks() {
    let text = "héllo\nwörld\n";
    let blocks = vec![
        blk(BlockKind::Paragraph, 2..3, 2..3), // inside a multibyte char
        blk(BlockKind::Paragraph, 1..1, 1..1), // unsorted
        blk(BlockKind::Paragraph, 100..200, 100..200), // past the end
    ];
    check_contract(text, &blocks);
}

// ---------------------------------------------------------------------------------------------
// Live preview, hand-built blocks

#[test]
fn heading_markers_hidden_unless_focused() {
    let text = "## Hello\n\n";
    let blocks = vec![blk(BlockKind::Heading { level: 2 }, 0..8, 3..8)];
    let cs = check_contract(text, &blocks);
    let t = Theme::light();
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    assert_eq!(text_where(&s, hidden), "## ");
    let big = text_where(&s, |f| f.font_id.size == t.body_size * 1.6);
    assert_eq!(big, "Hello");
    assert!(s.decorations.iter().any(|d| matches!(d, Decoration::Rule { range, .. } if *range == (3..8))));
    assert!(s.space_before > 0.0);

    let s = style(text, &blocks, &cs[0], Mode::LivePreview, true, &t);
    assert_eq!(text_where(&s, hidden), "");
    assert_eq!(text_where(&s, |f| f.color == t.marker), "## ");
}

#[test]
fn inline_spans() {
    //          0         1         2         3         4         5
    //          0123456789012345678901234567890123456789012345678901234567
    let text = "a **b** *c* ~~d~~ `e` [f](http://x) ![g](i.png) <http://y>";
    let mut p = blk(BlockKind::Paragraph, 0..text.len(), 0..text.len());
    p.inlines = vec![
        span(SpanKind::Strong, 2..7, 4..5),
        span(SpanKind::Emphasis, 8..11, 9..10),
        span(SpanKind::Strikethrough, 12..17, 14..15),
        span(SpanKind::Code, 18..21, 19..20),
        span(SpanKind::Link { url: "http://x".into() }, 22..35, 23..24),
        span(SpanKind::Image { url: "i.png".into(), alt: "g".into() }, 36..47, 38..39),
        span(SpanKind::Autolink { url: "http://y".into() }, 48..58, 49..57),
    ];
    let blocks = vec![p];
    let cs = check_contract(text, &blocks);
    let t = Theme::light();
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    assert_eq!(visible_text(&s), "a b c d e f g http://y");
    assert_eq!(text_where(&s, |f| f.italics && !hidden(f)), "cg");
    assert_eq!(text_where(&s, |f| f.strikethrough.width > 0.0), "d");
    assert_eq!(text_where(&s, |f| f.font_id.family == FontFamily::Monospace && !hidden(f)), "e");
    assert_eq!(text_where(&s, |f| f.color == t.link && !hidden(f)), "fhttp://y");
    assert_eq!(text_where(&s, |f| f.color == t.heading && !hidden(f)), "b");
    assert!(s.decorations.contains(&Decoration::InlineBg { range: 19..20, fill: t.inline_code_bg }));
    assert!(s.decorations.contains(&Decoration::Link { range: 23..24, url: "http://x".into() }));
    assert!(s.decorations.contains(&Decoration::Link { range: 49..57, url: "http://y".into() }));
    assert!(s.decorations.contains(&Decoration::Image { range: 36..47, url: "i.png".into(), alt: "g".into() }));

    // Focused: everything visible.
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, true, &t);
    assert_eq!(visible_text(&s), text);
}

#[test]
fn bold_families_when_registered() {
    let text = "**b** *i*";
    let mut p = blk(BlockKind::Paragraph, 0..9, 0..9);
    p.inlines = vec![span(SpanKind::Strong, 0..5, 2..3), span(SpanKind::Emphasis, 6..9, 7..8)];
    let blocks = vec![p];
    let mut t = Theme::light();
    t.font_families = true;
    let cs = chunks(text, &blocks);
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    let fam = |name: &str| text_where(&s, |f| f.font_id.family == FontFamily::Name(name.into()));
    assert_eq!(fam(BOLD_FAMILY), "b");
    assert_eq!(fam(ITALIC_FAMILY), "i");
    assert_eq!(text_where(&s, |f| f.italics), "");
}

fn code_doc() -> (String, Vec<Block>) {
    let text = "```rust\nfn main() {\n\tlet x = \"é\";\n}\n```\n\nafter\n".to_string();
    let open = 0..7;
    let close_start = text.find("\n```\n").map(|p| p + 1).unwrap_or(0);
    let close = close_start..close_start + 3;
    let content = 8..close_start;
    let info = CodeBlockInfo {
        fence: Some(Fence { ch: '`', len: 3, indent: 0 }),
        info: "rust".into(),
        lang: Some("rust".into()),
        closed: true,
        open_fence: Some(open),
        close_fence: Some(close.clone()),
    };
    let blocks = vec![
        blk(BlockKind::CodeBlock(info), 0..close.end, content),
        blk(BlockKind::Paragraph, close.end + 2..close.end + 7, close.end + 2..close.end + 7),
    ];
    (text, blocks)
}

#[test]
fn code_block_card_and_highlight() {
    let (text, blocks) = code_doc();
    let cs = check_contract(&text, &blocks);
    assert_eq!(cs.len(), 2);
    let t = Theme::light();
    let s = style(&text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    let content = blocks[0].content.clone();
    assert!(s.decorations.iter().any(|d| matches!(d, Decoration::Card { range, .. } if *range == blocks[0].range)));
    assert!(s.decorations.contains(&Decoration::CodeHeader { range: 0..7, lang: "rust".into(), code: content.clone() }));
    // Fences + their newlines hidden.
    assert_eq!(text_where(&s, hidden), "```rust\n```\n");
    // Code is monospace, highlighted with several colours, tab kept.
    let mut colours = std::collections::HashSet::new();
    for sec in &s.job.sections {
        if !hidden(&sec.format) && br(sec).start >= content.start && br(sec).end <= content.end {
            assert_eq!(sec.format.font_id.family, FontFamily::Monospace);
            assert_eq!(sec.format.font_id.size, t.code_size);
            colours.insert(sec.format.color);
        }
    }
    assert!(colours.len() >= 2, "{colours:?}");
    assert!(s.job.text.contains('\t'));

    let s = style(&text, &blocks, &cs[0], Mode::LivePreview, true, &t);
    assert_eq!(text_where(&s, hidden), "");
    assert_eq!(text_where(&s, |f| f.color == t.marker), "```rust\n```\n");
}

#[test]
fn indented_code() {
    let text = "    let a = 1;\n    b\n";
    let blocks = mdedit_syntax::parse(text);
    let cs = check_contract(text, &blocks);
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &Theme::dark());
    // The 4-space indents are syntax, not code: hidden in live preview so the code lines up
    // with the card padding.
    assert_eq!(text_where(&s, hidden), "        ");
    assert_eq!(visible_text(&s), "let a = 1;\nb");
    assert!(s.decorations.iter().any(|d| matches!(d, Decoration::CodeHeader { .. })));
}

#[test]
fn unclosed_fence() {
    let text = "```py\nx = 1\n";
    let info = CodeBlockInfo {
        fence: Some(Fence { ch: '`', len: 3, indent: 0 }),
        info: "py".into(),
        lang: Some("python".into()),
        closed: false,
        open_fence: Some(0..5),
        close_fence: None,
    };
    let blocks = vec![blk(BlockKind::CodeBlock(info), 0..11, 6..11)];
    let cs = check_contract(text, &blocks);
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &Theme::light());
    assert_eq!(text_where(&s, hidden), "```py\n");
}

#[test]
fn list_with_tasks_and_nesting() {
    //          0          1          2
    //          0123456 7890123456 789012345
    let text = "- [ ] a\n- [x] b\n  - c\n1. d\n";
    let item = |r: Range<usize>, task: Option<(bool, Range<usize>)>| {
        let (t, cb) = match task {
            Some((c, r)) => (Some(c), Some(r)),
            None => (None, None),
        };
        blk(BlockKind::ListItem { task: t, checkbox: cb }, r.clone(), r)
    };
    let inner_item = item(18..21, None);
    let mut inner = blk(BlockKind::List { ordered: false, start: 1 }, 18..21, 18..21);
    inner.children = vec![inner_item];
    let i1 = item(0..7, Some((false, 2..5)));
    let mut i2 = item(8..21, Some((true, 10..13)));
    i2.children = vec![inner];
    let mut list = blk(BlockKind::List { ordered: false, start: 1 }, 0..21, 0..21);
    list.children = vec![i1, i2];
    let ol_item = item(22..26, None);
    let mut ol = blk(BlockKind::List { ordered: true, start: 1 }, 22..26, 22..26);
    ol.children = vec![ol_item];
    let blocks = vec![list, ol];
    let cs = check_contract(text, &blocks);
    let t = Theme::light();
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    let d = &s.decorations;
    assert!(d.contains(&Decoration::Bullet { range: 0..1, depth: 1 }));
    assert!(d.contains(&Decoration::Bullet { range: 18..19, depth: 2 }), "{d:?}");
    assert!(d.contains(&Decoration::Checkbox { range: 2..5, checked: false }));
    assert!(d.contains(&Decoration::Checkbox { range: 10..13, checked: true }));
    // List markers and indentation take no room (the editor indents items and draws the
    // bullets in the indent); task boxes keep their width under the drawn checkbox.
    assert_eq!(text_where(&s, hidden), "- -   - ");
    assert_eq!(text_where(&s, |f| f.color == Color32::TRANSPARENT && !hidden(f)), "[ ][x]");

    let s = style(text, &blocks, &cs[0], Mode::LivePreview, true, &t);
    assert!(!s.decorations.iter().any(|d| matches!(d, Decoration::Bullet { .. } | Decoration::Checkbox { .. })));
    let s = style(text, &blocks, &cs[1], Mode::LivePreview, false, &t);
    assert_eq!(visible_text(&s), "d");
    assert!(s.decorations.contains(&Decoration::ListNumber { range: 0..2, text: "1.".into() }), "{:?}", s.decorations);
    // While editing, the number is plain text.
    let s = style(text, &blocks, &cs[1], Mode::LivePreview, true, &t);
    assert_eq!(visible_text(&s), "1. d");
}

#[test]
fn block_quote_nested() {
    let text = "> a\n> > b\n";
    let mut inner_p = blk(BlockKind::Paragraph, 8..9, 8..9);
    inner_p.inlines.clear();
    let mut inner = blk(BlockKind::BlockQuote, 6..9, 6..9);
    inner.children = vec![inner_p];
    let mut q = blk(BlockKind::BlockQuote, 0..9, 0..9);
    q.children = vec![blk(BlockKind::Paragraph, 2..3, 2..3), inner];
    let blocks = vec![q];
    let cs = check_contract(text, &blocks);
    let t = Theme::light();
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    assert!(s.decorations.contains(&Decoration::QuoteBar { range: 0..9, color: t.quote_bar, depth: 1 }));
    assert!(s.decorations.contains(&Decoration::QuoteBar { range: 6..9, color: t.quote_bar, depth: 2 }));
    assert_eq!(text_where(&s, |f| f.color == Color32::TRANSPARENT), "> > > ");
    assert_eq!(text_where(&s, |f| f.color == t.muted), "a\nb");
}

#[test]
fn thematic_break_and_table() {
    let text = "---\n\n| a | b |\n|---|:-:|\n| 1 | 2 |\n";
    let rows = vec![
        TableRow { header: true, range: 5..14, cells: vec![7..8, 11..12] },
        TableRow { header: false, range: 25..34, cells: vec![27..28, 31..32] },
    ];
    let blocks = vec![
        blk(BlockKind::ThematicBreak, 0..3, 0..3),
        blk(BlockKind::Table { aligns: vec![mdedit_syntax::Align::None; 2], rows }, 5..34, 5..34),
    ];
    let cs = check_contract(text, &blocks);
    let t = Theme::light();
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    assert!(s.decorations.contains(&Decoration::Rule { range: 0..3, color: t.rule }));
    assert_eq!(text_where(&s, |f| f.color == Color32::TRANSPARENT), "---");
    let s = style(text, &blocks, &cs[1], Mode::LivePreview, false, &t);
    assert!(s.decorations.contains(&Decoration::TableGrid { rows: vec![vec![2..3, 6..7], vec![22..23, 26..27]], aligns: vec![mdedit_syntax::Align::None; 2], color: t.table_border }));
    assert_eq!(text_where(&s, |f| f.color == t.heading), " a  b ");
    // Unfocused: delimiter row collapsed, pipes transparent at full size.
    assert_eq!(text_where(&s, hidden), "|---|:-:|\n");
    assert_eq!(text_where(&s, |f| f.color == Color32::TRANSPARENT && !hidden(f)), "||||||");
    let s = style(text, &blocks, &cs[1], Mode::LivePreview, true, &t);
    assert_eq!(text_where(&s, hidden), "");
    assert!(text_where(&s, |f| f.color == t.marker).contains("|---|:-:|"));
}

#[test]
fn front_matter_and_math() {
    let text = "---\ntitle: x\n---\n\n$$\na^2\n$$\n";
    let blocks = vec![
        blk(BlockKind::FrontMatter, 0..16, 0..16),
        blk(BlockKind::MathBlock, 18..28, 21..24),
    ];
    let cs = check_contract(text, &blocks);
    let t = Theme::light();
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    assert_eq!(text_where(&s, hidden), "---\n---");
    assert!(matches!(s.decorations[0], Decoration::Card { .. }));
    let s = style(text, &blocks, &cs[1], Mode::LivePreview, false, &t);
    assert_eq!(text_where(&s, hidden), "$$\n\n$$");
}

#[test]
fn crlf_cr_hidden() {
    let text = "# A\r\n\r\nb\r\n";
    let blocks = vec![blk(BlockKind::Heading { level: 1 }, 0..3, 2..3), blk(BlockKind::Paragraph, 7..8, 7..8)];
    let cs = check_contract(text, &blocks);
    let s = style(text, &blocks, &cs[0], Mode::Source, false, &Theme::light());
    assert_eq!(text_where(&s, hidden), "\r\r");
}

// ---------------------------------------------------------------------------------------------
// Source mode

#[test]
fn source_mode_colours() {
    let (text, blocks) = code_doc();
    let cs = chunks(&text, &blocks);
    let t = Theme::dark();
    let s = style(&text, &blocks, &cs[0], Mode::Source, false, &t);
    assert!(s.decorations.is_empty());
    assert_eq!(visible_text(&s), s.job.text);
    assert_eq!(text_where(&s, |f| f.color == t.marker), "```rust\n```\n");
    assert!(s.job.sections.iter().all(|x| x.format.font_id.size == t.code_size));

    let text = "# H **b**\n";
    let mut h = blk(BlockKind::Heading { level: 1 }, 0..9, 2..9);
    h.inlines = vec![span(SpanKind::Strong, 4..9, 6..7)];
    let blocks = vec![h];
    let cs = chunks(text, &blocks);
    let s = style(text, &blocks, &cs[0], Mode::Source, false, &t);
    assert_eq!(text_where(&s, |f| f.color == t.marker), "# ****");
    assert_eq!(text_where(&s, |f| f.color == t.heading), "H b");
}

// ---------------------------------------------------------------------------------------------
// Property tests

fn arb_kind() -> impl Strategy<Value = BlockKind> {
    prop_oneof![
        Just(BlockKind::Paragraph),
        (0u8..8).prop_map(|level| BlockKind::Heading { level }),
        Just(BlockKind::BlockQuote),
        Just(BlockKind::ThematicBreak),
        any::<bool>().prop_map(|ordered| BlockKind::List { ordered, start: 1 }),
        Just(BlockKind::Html),
        Just(BlockKind::FrontMatter),
        Just(BlockKind::MathBlock),
        Just(BlockKind::CodeBlock(CodeBlockInfo {
            fence: Some(Fence { ch: '`', len: 3, indent: 0 }),
            info: "rust".into(),
            lang: Some("rust".into()),
            closed: true,
            open_fence: Some(0..3),
            close_fence: Some(5..9),
        })),
    ]
}

fn arb_span_kind() -> impl Strategy<Value = SpanKind> {
    prop_oneof![
        Just(SpanKind::Strong),
        Just(SpanKind::Emphasis),
        Just(SpanKind::Code),
        Just(SpanKind::Link { url: "u".into() }),
        Just(SpanKind::Image { url: "u".into(), alt: "a".into() }),
        Just(SpanKind::InlineMath),
        Just(SpanKind::FootnoteReference { label: "1".into() }),
        Just(SpanKind::HardBreak),
        Just(SpanKind::Html),
    ]
}

const MD_CHARS: &str = "[a-z #*_`>\\-+|\\[\\]()!$\\n\\t\\ré日🎉~:0-9]{0,160}";

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Random text with random (possibly nonsensical) blocks and spans: contract holds, no panics.
    #[test]
    fn contract_random_blocks(
        text in MD_CHARS,
        raw in proptest::collection::vec((arb_kind(), 0usize..200, 0usize..60, 0usize..200, 0usize..60,
            proptest::collection::vec((arb_span_kind(), 0usize..200, 0usize..30, 0usize..200, 0usize..30), 0..4)), 0..6),
    ) {
        let mut blocks: Vec<Block> = raw.into_iter().map(|(kind, s, l, cs, cl, spans)| {
            let mut kind = kind;
            if let BlockKind::CodeBlock(info) = &mut kind {
                info.open_fence = Some(s..s + 3);
                info.close_fence = Some(s + l..s + l + 3);
            }
            let mut b = blk(kind, s..s + l, cs..cs + cl);
            b.inlines = spans.into_iter().map(|(k, a, al, c, cl)| span(k, a..a + al, c..c + cl)).collect();
            if l > 2 {
                b.children.push(blk(BlockKind::ListItem { task: Some(true), checkbox: Some(s + 1..s + 2) }, s..s + l / 2, s..s + l / 2));
            }
            b
        }).collect();
        blocks.sort_by_key(|b| b.range.start);
        check_contract(&text, &blocks);
    }

    /// Whatever the real parser returns (empty while it is a stub) obeys the contract.
    #[test]
    fn contract_with_parser(text in MD_CHARS) {
        let blocks = mdedit_syntax::parse(&text);
        check_contract(&text, &blocks);
    }
}

// ---------------------------------------------------------------------------------------------
// With the real parser.

#[test]
fn parsed_document_contract() {
    let text = "---\ntitle: t\n---\n\n# Title\n\nSome **bold** and `code` and [a link](http://x).\n\n\
                > quote\n> > nested\n\n- [ ] task\n- [x] done\n  - nested\n\n1. one\n2. two\n\n\
                ```rust\nfn main() {\n\tprintln!(\"é\");\n}\n```\n\n    indented\n\n---\n\n\
                | a | b |\n|---|---|\n| 1 | 2 |\n\n$$\nx^2\n$$\n\n<div>html</div>\n\n[^1]: note\n\n\
                Setext\n======\n\nCRLF line\r\nend\r\n```unclosed\nstuff";
    let blocks = mdedit_syntax::parse(text);
    check_contract(text, &blocks);
}

#[test]
fn parsed_heading_and_code_hidden() {
    let text = "# Title\n\n```rust\nfn main() {}\n```\n";
    let blocks = mdedit_syntax::parse(text);
    let cs = check_contract(text, &blocks);
    assert_eq!(cs.len(), 2);
    let t = Theme::light();
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    assert_eq!(text_where(&s, hidden), "# ");
    let s = style(text, &blocks, &cs[1], Mode::LivePreview, false, &t);
    assert_eq!(text_where(&s, hidden), "```rust\n```");
    assert!(s.decorations.iter().any(|d| matches!(d, Decoration::Card { .. })));
    assert!(s.decorations.iter().any(|d| matches!(d, Decoration::CodeHeader { lang, .. } if lang == "rust")));
}

#[test]
fn parsed_inlines_hidden() {
    let text = "a **b** `c` [d](http://e)\n";
    let blocks = mdedit_syntax::parse(text);
    let cs = check_contract(text, &blocks);
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &Theme::light());
    assert_eq!(visible_text(&s), "a b c d");
}

#[test]
fn style_is_cheap_for_big_doc() {
    // 1 MB of paragraphs; styling one chunk must not scan the whole block list linearly.
    let para = "Lorem ipsum dolor sit amet.\n\n";
    let n = 1_000_000 / para.len();
    let text = para.repeat(n);
    let blocks: Vec<Block> = (0..n)
        .map(|i| {
            let s = i * para.len();
            blk(BlockKind::Paragraph, s..s + para.len() - 2, s..s + para.len() - 2)
        })
        .collect();
    let cs = chunks(&text, &blocks);
    assert_eq!(cs.len(), n);
    let t = Theme::light();
    let started = std::time::Instant::now();
    for c in cs.iter().step_by(n / 100) {
        let s = style(&text, &blocks, c, Mode::LivePreview, false, &t);
        assert_eq!(s.job.text, &text[c.range.start..c.range.end - 1]);
    }
    assert!(started.elapsed() < std::time::Duration::from_millis(200), "{:?}", started.elapsed());
}


#[test]
fn link_underline_uses_natural_line_height() {
    let text = "see [x](u) now";
    let mut p = blk(BlockKind::Paragraph, 0..14, 0..14);
    p.inlines = vec![span(SpanKind::Link { url: "u".into() }, 4..10, 5..6)];
    let blocks = vec![p];
    let cs = chunks(text, &blocks);
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &Theme::light());
    for sec in &s.job.sections {
        let f = &sec.format;
        if f.underline != Stroke::NONE {
            assert_eq!(&s.job.text[br(sec)], "x");
            assert_eq!(f.line_height, None);
            assert_eq!(f.valign, egui::Align::Center);
        }
    }
    assert_eq!(text_where(&s, |f| f.underline != Stroke::NONE), "x");
}

#[test]
fn done_task_strike_only_on_text() {
    let text = "- [x] foo bar\n  - [ ] sub\n";
    let sub = blk(BlockKind::ListItem { task: Some(false), checkbox: Some(18..21) }, 16..25, 16..25);
    let mut sub_list = blk(BlockKind::List { ordered: false, start: 1 }, 16..25, 16..25);
    sub_list.children = vec![sub];
    let mut item = blk(BlockKind::ListItem { task: Some(true), checkbox: Some(2..5) }, 0..25, 0..25);
    item.children = vec![blk(BlockKind::Paragraph, 6..13, 6..13), sub_list];
    let mut list = blk(BlockKind::List { ordered: false, start: 1 }, 0..25, 0..25);
    list.children = vec![item];
    let blocks = vec![list];
    let cs = check_contract(text, &blocks);
    for focused in [false, true] {
        let s = style(text, &blocks, &cs[0], Mode::LivePreview, focused, &Theme::light());
        assert_eq!(text_where(&s, |f| f.strikethrough != Stroke::NONE), "foo bar");
    }
}

#[test]
fn code_in_quote_highlights_without_prefixes() {
    let text = "> ```rust\n> fn main() { let x = 1; }\n> ```\n";
    let blocks = mdedit_syntax::parse(text);
    let cs = check_contract(text, &blocks);
    let t = Theme::light();
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    // The second line's `> ` is transparent (quote marker), not highlighter-coloured.
    let line2 = text.find("\n> fn").map(|p| p + 1).unwrap_or(0);
    let mut colours = std::collections::HashSet::new();
    for sec in &s.job.sections {
        let r = br(sec);
        if r.start < line2 + 2 && r.end > line2 {
            assert!(r.end <= line2 + 2 || sec.format.color == Color32::TRANSPARENT || r.start >= line2 + 2);
        }
        if r.start >= line2 + 2 && r.end <= line2 + 35 && !hidden(&sec.format) {
            colours.insert(sec.format.color);
        }
    }
    assert!(colours.len() >= 2, "{colours:?}");
    // The `> ` prefixes are hidden in the code (the editor offsets the card inside the quote).
    assert_eq!(text_where(&s, |f| f.color == Color32::TRANSPARENT && !hidden(f)), "");
    // The highlighter saw the code without prefixes.
    let b = &blocks[0].children[0];
    let lines = mdedit_syntax::code_lines(text, b);
    assert_eq!(lines.len(), 1);
    assert_eq!(&text[lines[0].clone()], "fn main() { let x = 1; }");
}

#[test]
fn bold_family_on_headings_and_strong() {
    let text = "# Head **x**\n";
    let blocks = mdedit_syntax::parse(text);
    let mut t = Theme::light();
    t.font_families = true;
    let cs = chunks(text, &blocks);
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    assert_eq!(text_where(&s, |f| f.font_id.family == FontFamily::Name(BOLD_FAMILY.into())), "Head x");
    let text = "a **b** c";
    let blocks = mdedit_syntax::parse(text);
    let cs = chunks(text, &blocks);
    let s = style(text, &blocks, &cs[0], Mode::LivePreview, false, &t);
    assert_eq!(text_where(&s, |f| f.font_id.family == FontFamily::Name(BOLD_FAMILY.into())), "b");
}

#[test]
fn deep_block_nesting_no_stack_overflow() {
    // Hand-built 10 000-deep quote chain (independent of any parser depth cap).
    let n = 10_000;
    let text = format!("{}x\n", ">".repeat(n));
    let mut b = blk(BlockKind::Paragraph, n..n + 1, n..n + 1);
    for i in (0..n).rev() {
        let mut q = blk(BlockKind::BlockQuote, i..n + 1, i..n + 1);
        q.children = vec![b];
        b = q;
    }
    let blocks = vec![b];
    let cs = chunks(&text, &blocks);
    let s = std::thread::Builder::new()
        .stack_size(1 << 20) // 1 MiB: deep recursion would overflow this
        .spawn(move || {
            let s = style(&text, &blocks, &cs[0], Mode::LivePreview, false, &Theme::light());
            let ok = s.job.text == text[..text.len() - 1];
            // Dropping a 10k-deep Block tree recurses too; leak it to keep the test about style.
            std::mem::forget(blocks);
            ok
        })
        .and_then(|h| h.join().map_err(|_| std::io::Error::other("panicked")));
    assert!(matches!(s, Ok(true)));
    // And via the parser, whatever its own cap.
    let text = format!("{}x\n", ">".repeat(n));
    let blocks = mdedit_syntax::parse(&text);
    check_contract_live(&text, &blocks);
}

fn check_contract_live(text: &str, blocks: &[Block]) {
    for c in chunks(text, blocks) {
        let s = style(text, blocks, &c, Mode::LivePreview, false, &Theme::light());
        let src = &text[c.range.clone()];
        assert_eq!(s.job.text, src.strip_suffix('\n').unwrap_or(src));
    }
}

#[test]
fn deep_inline_nesting_is_fast() {
    let n = 100_000;
    let text = format!("{}x{}", "*".repeat(n), "*".repeat(n));
    let blocks = mdedit_syntax::parse(&text);
    let cs = chunks(&text, &blocks);
    let started = std::time::Instant::now();
    for c in &cs {
        let s = style(&text, &blocks, c, Mode::LivePreview, false, &Theme::light());
        assert_eq!(s.job.text.len(), c.range.len() - usize::from(text[c.range.clone()].ends_with('\n')));
    }
    let took = started.elapsed();
    if !cfg!(debug_assertions) {
        assert!(took < std::time::Duration::from_millis(500), "{took:?}");
    }
    // Hand-built: 50 000 nested emphasis spans.
    let m = 50_000;
    let text = format!("{}x{}", "*".repeat(m), "*".repeat(m));
    let mut p = blk(BlockKind::Paragraph, 0..text.len(), 0..text.len());
    p.inlines = (0..m).map(|i| span(SpanKind::Emphasis, i..text.len() - i, i + 1..text.len() - i - 1)).collect();
    let blocks = vec![p];
    let cs = chunks(&text, &blocks);
    let started = std::time::Instant::now();
    let _ = style(&text, &blocks, &cs[0], Mode::LivePreview, false, &Theme::light());
    let took = started.elapsed();
    if !cfg!(debug_assertions) {
        assert!(took < std::time::Duration::from_millis(500), "{took:?}");
    }
}

#[test]
fn chunk_inside_a_list_has_no_outside_decorations() {
    // The editor gives a code block nested in a list a chunk of its own; the list's other
    // parts must not leave empty (phantom) decorations at its edges, and vice versa.
    let text = "- one\n\n  ```bash\n  x\n  ```\n\n- two\n";
    let blocks = mdedit_syntax::parse(text);
    let t = Theme::dark();
    let a = text.find("  ```").unwrap();
    let b = text.find("- two").unwrap();
    for (r, want_card, want_bullets) in [(0..a, false, 1), (a..b, true, 0), (b..text.len(), false, 1)] {
        let chunk = Chunk { range: r.clone(), block: Some(0) };
        let s = style(text, &blocks, &chunk, Mode::LivePreview, false, &t);
        let cards = s.decorations.iter().filter(|d| matches!(d, Decoration::Card { .. })).count();
        let bullets = s.decorations.iter().filter(|d| matches!(d, Decoration::Bullet { .. })).count();
        assert_eq!((cards > 0, bullets), (want_card, want_bullets), "{r:?}: {:?}", s.decorations);
    }
}
