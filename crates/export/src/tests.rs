use super::*;

fn frag(md: &str) -> String {
    to_html_fragment(md, &Highlighter::new())
}

fn doc(md: &str, raw: bool) -> String {
    let o = HtmlOptions { title: "T <x>".into(), theme: HlTheme::Dark, allow_raw_html: raw };
    to_html(md, &o, &Highlighter::new())
}

#[test]
fn standalone_document() {
    let h = doc("# Hi\n\ntext", false);
    assert!(h.starts_with("<!DOCTYPE html>"));
    assert!(h.contains("<meta charset=\"utf-8\">"));
    assert!(h.contains("name=\"viewport\""));
    assert!(h.contains("<title>T &lt;x&gt;</title>"));
    assert!(h.contains("color-scheme:dark"));
    assert!(h.contains("max-width:50rem"));
    assert!(h.contains("<h1 id=\"hi\">Hi</h1>"));
    assert!(h.contains("<p>text</p>"));
    assert!(h.trim_end().ends_with("</html>"));
    let light = to_html("x", &HtmlOptions { title: String::new(), theme: HlTheme::Light, allow_raw_html: false }, &Highlighter::new());
    assert!(light.contains("color-scheme:light"));
    assert!(light.contains("<title>Untitled</title>"));
    let titled = to_html("# First *one*\n", &HtmlOptions { title: " ".into(), theme: HlTheme::Light, allow_raw_html: false }, &Highlighter::new());
    assert!(titled.contains("<title>First one</title>"));
}

#[test]
fn code_blocks() {
    let h = frag("```rs title=x\nfn main() {\n\tlet a = 1 < 2;\n}\n```\n\n```\nplain & <b>\n```\n\n    indented\n");
    assert!(h.contains("<pre><code class=\"language-rust\">"), "{h}");
    assert!(h.contains("\tlet a = 1 &lt; 2;") || h.contains("&lt;"), "{h}");
    assert!(h.contains("<pre><code>plain &amp; &lt;b&gt;\n</code></pre>"), "{h}");
    assert!(h.contains("<pre><code>indented\n</code></pre>"), "{h}");
    // tabs preserved
    assert!(h.contains('\t'));
}

#[test]
fn two_code_blocks_stay_separate() {
    let h = frag("```python\nprint(1)\n```\n\n```bash\necho 2\n```\n");
    assert_eq!(h.matches("<pre>").count(), 2);
    assert!(h.contains("language-python"));
    assert!(h.contains("language-bash"));
}

#[test]
fn highlight_spans_survive_sanitizer() {
    let s = sanitize_html("<pre><code class=\"language-rust\"><span style=\"color:#ff0000;font-weight:bold\">fn</span></code></pre>");
    assert!(s.contains("style=\"color:#ff0000;font-weight:bold\""), "{s}");
    assert!(s.contains("class=\"language-rust\""));
    let s = sanitize_html("<span style=\"background:url(http://evil)\">x</span>");
    assert!(!s.contains("evil"), "{s}");
}

#[test]
fn tasks_tables_math_footnotes() {
    let h = frag("- [ ] a\n- [x] b\n\n| l | c | r |\n|:--|:-:|--:|\n| 1 | 2 | 3 |\n\n$x^2$ and $$y$$\n\nNote[^n].\n\n[^n]: The note.\n");
    assert!(h.contains("<input disabled=\"\" type=\"checkbox\"") || h.contains("type=\"checkbox\""), "{h}");
    assert!(h.contains("checked"), "{h}");
    assert_eq!(h.matches("type=\"checkbox\"").count(), 2);
    assert!(h.contains("<table>"));
    assert!(h.contains("<th style=\"text-align:left\">l</th>") || h.contains("text-align:left"), "{h}");
    assert!(h.contains("text-align:center"));
    assert!(h.contains("<td style=\"text-align:right\">3</td>"), "{h}");
    assert!(h.contains("<span class=\"math math-inline\">x^2</span>"), "{h}");
    assert!(h.contains("<span class=\"math math-display\">y</span>"));
    assert!(h.contains("href=\"#fn-n\""), "{h}");
    assert!(h.contains("id=\"fn-n\""), "{h}");
}

#[test]
fn inline_formatting_and_links() {
    let h = frag("**b** *i* ~~s~~ `c<` [l](https://x.org \"t\") ![a *b*](img.png) <me@x.org>\nline  \nbreak");
    assert!(h.contains("<strong>b</strong>"));
    assert!(h.contains("<em>i</em>"));
    assert!(h.contains("<del>s</del>"));
    assert!(h.contains("<code>c&lt;</code>"));
    assert!(h.contains("href=\"https://x.org\""), "{h}");
    assert!(h.contains("title=\"t\""));
    assert!(h.contains("<img src=\"img.png\" alt=\"a b\""), "{h}");
    assert!(h.contains("href=\"mailto:me@x.org\""), "{h}");
    assert!(h.contains("<br>"), "{h}");
}

#[test]
fn front_matter_not_rendered() {
    let h = frag("---\nsecret: yes\n---\n\n# Body\n");
    assert!(!h.contains("secret"), "{h}");
    assert!(h.contains("Body"));
    let h = frag("text\n\n---\nkey: v\n---\n");
    assert!(h.contains("key: v"), "{h}");
}

#[test]
fn heading_ids_unique() {
    let h = frag("# A b\n\n# A b\n\n## X {#custom .cls}\n");
    assert!(h.contains("id=\"a-b\""));
    assert!(h.contains("id=\"a-b-1\""));
    assert!(h.contains("id=\"custom\""), "{h}");
    assert!(h.contains("class=\"cls\""), "{h}");
}

#[test]
fn alerts() {
    let h = frag("> [!WARNING]\n> Careful\n");
    assert!(h.contains("markdown-alert-warning"), "{h}");
    assert!(h.contains("Careful"));
}

// ----- XSS -----

#[test]
fn xss_script_tag() {
    let h = frag("hello <script>alert(1)</script> world\n\n<script>\nalert(2)\n</script>\n");
    assert!(!h.contains("<script"), "{h}");
    assert!(!h.contains("alert(2)"), "{h}");
    let d = doc("<script>alert(1)</script>", false);
    assert!(!d.contains("<script"));
}

#[test]
fn xss_event_handlers() {
    let h = frag("<img src=x onerror=alert(1)>\n\n<a href=\"#\" onclick=\"alert(1)\">x</a> <div onmouseover=\"x()\">d</div>");
    assert!(!h.contains("onerror"), "{h}");
    assert!(!h.contains("onclick"), "{h}");
    assert!(!h.contains("onmouseover"), "{h}");
}

#[test]
fn xss_javascript_urls() {
    for md in [
        "[x](javascript:alert(1))",
        "[x](JavaScript:alert(1))",
        "[x](java\tscript:alert(1))",
        "[x](  javascript:alert(1))",
        "[x](vbscript:msgbox)",
        "[x](data:text/html;base64,PHNjcmlwdD4=)",
        "<a href=\"javascript:alert(1)\">x</a>",
        "<a href=\"jav&#x09;ascript:alert(1)\">x</a>",
        "[x]: javascript:alert(1)\n\n[x]",
        "<javascript:alert(1)>",
        "![i](javascript:alert(1))",
    ] {
        let h = frag(md);
        let lower = h.to_ascii_lowercase();
        assert!(!lower.contains("=\"javascript:"), "{md} -> {h}");
        assert!(!lower.contains("=\"jav"), "{md} -> {h}");
        assert!(!lower.contains("=\"vbscript:"), "{md} -> {h}");
        assert!(!lower.contains("=\"data:text"), "{md} -> {h}");
        assert!(!lower.contains("href=\"data"), "{md} -> {h}");
    }
    // even with raw HTML allowed, Markdown link URLs are filtered
    let d = doc("[x](javascript:alert(1))", true);
    assert!(!d.contains("href=\"javascript:"));
}

#[test]
fn data_images_allowed_only_for_images() {
    let h = frag("![p](data:image/png;base64,iVBORw0KGgo=)");
    assert!(h.contains("src=\"data:image/png;base64,iVBORw0KGgo=\""), "{h}");
    let h = frag("[p](data:image/png;base64,AAAA)");
    assert!(!h.contains("data:"), "{h}");
}

#[test]
fn raw_html_allowed_when_requested() {
    let d = doc("<div class=\"x\" onclick=\"y\">raw</div>", true);
    assert!(d.contains("onclick"));
    let d = doc("<div class=\"x\" onclick=\"y\">raw</div>", false);
    assert!(!d.contains("onclick"));
    assert!(d.contains("raw"));
}

#[test]
fn safe_url_cases() {
    assert!(safe_url("https://a", false).is_some());
    assert!(safe_url("./rel/path.md", false).is_some());
    assert!(safe_url("#frag", false).is_some());
    assert!(safe_url("/abs", false).is_some());
    assert!(safe_url("dir/a:b", false).is_some());
    assert!(safe_url("mailto:x@y", false).is_some());
    assert!(safe_url("javascript:x", false).is_none());
    assert!(safe_url("file:///etc/passwd", false).is_none());
    assert!(safe_url("data:image/png;base64,x", true).is_some());
    assert!(safe_url("data:image/png;base64,x", false).is_none());
}

mod props {
    use super::*;
    use proptest::prelude::*;
    proptest! {
        #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]
        #[test]
        fn export_never_panics(s in "\\PC{0,300}") {
            let h = frag(&s);
            prop_assert!(!h.to_ascii_lowercase().contains("<script"));
        }
    }
}

#[test]
fn deep_nesting_does_not_crash() {
    let h = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(|| {
            for t in [format!("{} x", ">".repeat(10_000)), format!("{}x", "> - ".repeat(5_000))] {
                let out = frag(&t);
                assert!(out.contains('x'));
            }
        })
        .expect("spawn");
    h.join().expect("export overflowed the stack on deep nesting");
}
