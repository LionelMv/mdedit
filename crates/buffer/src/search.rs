//! Find in text. STABLE signatures.

use std::ops::Range;

/// Maximum number of matches returned by [`find_all`].
pub const MAX_MATCHES: usize = 100_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Build the regex used for `query` under `opts`. `^`/`$` match at line boundaries in regex
/// mode. (Added API: lets callers reuse the compiled regex, e.g. for replace-all.)
pub fn build_regex(query: &str, opts: &SearchOptions) -> Result<regex::Regex, String> {
    let mut pattern = if opts.regex { format!("(?:{query})") } else { regex::escape(query) };
    if opts.whole_word {
        // For plain queries only add `\b` on a side that starts/ends with a word char, so a
        // query like `#tag` still matches as a whole word.
        let (pre, post) = if opts.regex {
            (true, true)
        } else {
            (query.chars().next().is_some_and(is_word_char), query.chars().next_back().is_some_and(is_word_char))
        };
        pattern = format!("{}{pattern}{}", if pre { r"\b" } else { "" }, if post { r"\b" } else { "" });
    }
    regex::RegexBuilder::new(&pattern)
        .case_insensitive(!opts.case_sensitive)
        .multi_line(true)
        .size_limit(32 * 1024 * 1024)
        .build()
        .map_err(|e| match e {
            regex::Error::Syntax(msg) => msg,
            regex::Error::CompiledTooBig(_) => "regular expression is too large".to_owned(),
            other => other.to_string(),
        })
}

/// All non-overlapping matches of `query` in `text`. An invalid regex returns `Err(message)`.
/// An empty query returns no matches. Capped at 100 000 matches. Empty-width matches are
/// skipped.
pub fn find_all(text: &str, query: &str, opts: &SearchOptions) -> Result<Vec<Range<usize>>, String> {
    find_up_to(text, query, opts, MAX_MATCHES)
}

/// [`find_all`] with an explicit cap (replace-all must see every match).
pub fn find_up_to(text: &str, query: &str, opts: &SearchOptions, limit: usize) -> Result<Vec<Range<usize>>, String> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let re = build_regex(query, opts)?;
    Ok(re.find_iter(text).filter(|m| !m.range().is_empty()).take(limit).map(|m| m.range()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(case_sensitive: bool, whole_word: bool, regex: bool) -> SearchOptions {
        SearchOptions { case_sensitive, whole_word, regex }
    }

    #[test]
    fn plain_case_insensitive_unicode() {
        let t = "Straße STRASSE straße ÉCOLE école";
        assert_eq!(find_all(t, "straße", &opts(false, false, false)).unwrap().len(), 2);
        assert_eq!(find_all(t, "école", &opts(false, false, false)).unwrap().len(), 2);
        assert_eq!(find_all(t, "école", &opts(true, false, false)).unwrap().len(), 1);
    }

    #[test]
    fn plain_escapes_metachars() {
        let t = "a.b axb (x) [y]";
        assert_eq!(find_all(t, "a.b", &opts(true, false, false)).unwrap(), vec![0..3]);
        assert_eq!(find_all(t, "(x)", &opts(true, false, false)).unwrap(), vec![8..11]);
        assert_eq!(find_all(t, "[", &opts(true, false, false)).unwrap(), vec![12..13]);
    }

    #[test]
    fn whole_word() {
        let t = "cat concat cat_x cat. #tag x#tag";
        assert_eq!(find_all(t, "cat", &opts(true, true, false)).unwrap(), vec![0..3, 17..20]);
        assert_eq!(find_all(t, "#tag", &opts(true, true, false)).unwrap(), vec![22..26, 28..32]);
    }

    #[test]
    fn regex_mode() {
        let t = "a1 b22\nc333";
        assert_eq!(find_all(t, r"\d+", &opts(true, false, true)).unwrap(), vec![1..2, 4..6, 8..11]);
        assert_eq!(find_all(t, r"^\w", &opts(true, false, true)).unwrap(), vec![0..1, 7..8]);
        assert!(find_all(t, "(", &opts(true, false, true)).is_err());
        // Empty-width matches skipped.
        assert_eq!(find_all(t, "x*", &opts(true, false, true)).unwrap(), Vec::<Range<usize>>::new());
        // Alternation stays grouped with whole-word.
        assert_eq!(find_all("ab a b", "a|b", &opts(true, true, true)).unwrap(), vec![3..4, 5..6]);
    }

    #[test]
    fn empty_query_and_cap() {
        assert!(find_all("abc", "", &SearchOptions::default()).unwrap().is_empty());
        let big = "a".repeat(MAX_MATCHES + 10);
        assert_eq!(find_all(&big, "a", &SearchOptions::default()).unwrap().len(), MAX_MATCHES);
    }

    proptest::proptest! {
        #[test]
        fn plain_matches_are_real(text in "[aAbB \u{e9}\u{c9}]{0,40}", q in "[ab\u{e9}]{1,3}") {
            let hits = find_all(&text, &q, &opts(true, false, false)).unwrap();
            let naive: Vec<_> = text.match_indices(q.as_str()).map(|(i, m)| i..i + m.len()).collect();
            proptest::prop_assert_eq!(hits, naive);
            for r in find_all(&text, &q, &opts(false, false, false)).unwrap() {
                proptest::prop_assert_eq!(text[r].to_lowercase(), q.to_lowercase());
            }
        }
    }
}
