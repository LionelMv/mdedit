//! Link policy: what happens when the user Ctrl+clicks a link in a document.
//!
//! Security rule: we never execute anything. Web and mail links go to the desktop's handler
//! via `open` (which spawns `xdg-open` with an argument, never a shell); relative Markdown
//! links open in a tab; everything else is refused.

use std::path::{Path, PathBuf};

pub use mdedit_syntax::slugify;

#[derive(Debug, PartialEq, Eq)]
pub enum LinkAction {
    /// Hand to the desktop (http, https, mailto).
    External(String),
    /// Open this existing Markdown file in a tab.
    OpenDoc(PathBuf),
    /// Jump to the heading with this slug in the current document (`#section`).
    Anchor(String),
    /// Refused, with a reason for the toast.
    Refuse(String),
}

const DOC_EXTS: &[&str] = &["md", "markdown", "mdown", "mkd", "mkdn", "txt"];

pub fn is_markdown_path(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| DOC_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Decide what to do with `url` as written in a document located in `base_dir`.
pub fn classify(url: &str, base_dir: Option<&Path>) -> LinkAction {
    let url = url.trim().trim_start_matches('<').trim_end_matches('>').trim();
    if url.is_empty() {
        return LinkAction::Refuse("Empty link".into());
    }
    if url.chars().any(|c| c.is_control()) {
        return LinkAction::Refuse("Link contains control characters".into());
    }
    if let Some(anchor) = url.strip_prefix('#') {
        return LinkAction::Anchor(crate::cli::percent_decode(anchor).unwrap_or_else(|| anchor.to_string()));
    }
    if let Some(scheme) = scheme_of(url) {
        let lower = scheme.to_ascii_lowercase();
        return match lower.as_str() {
            "http" | "https" => {
                if url.len() > lower.len() + 3 && url[lower.len()..].starts_with("://") {
                    LinkAction::External(url.to_string())
                } else {
                    LinkAction::Refuse(format!("Malformed {lower} link"))
                }
            }
            "mailto" => match mailto_query_ok(url) {
                true => LinkAction::External(url.to_string()),
                false => LinkAction::Refuse("Refusing mailto link with unsupported fields (e.g. attach)".into()),
            },
            _ => LinkAction::Refuse(format!("Refusing to open \"{scheme}:\" links")),
        };
    }
    // A local path: strip query/fragment, decode %XX.
    let path_part = url.split(['#', '?']).next().unwrap_or("");
    if path_part.is_empty() {
        return LinkAction::Refuse("Empty link".into());
    }
    let decoded = crate::cli::percent_decode(path_part).unwrap_or_else(|| path_part.to_string());
    let path = Path::new(&decoded);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(stripped) = decoded.strip_prefix("~/") {
        match std::env::var_os("HOME") {
            Some(home) => Path::new(&home).join(stripped),
            None => return LinkAction::Refuse("Cannot resolve ~".into()),
        }
    } else if let Some(base) = base_dir {
        base.join(path)
    } else {
        return LinkAction::Refuse("Relative link in an unsaved document".into());
    };
    if !is_markdown_path(&resolved) {
        return LinkAction::Refuse(format!("Only Markdown files open from links: {}", resolved.display()));
    }
    match std::fs::metadata(&resolved) {
        Ok(m) if m.is_file() => LinkAction::OpenDoc(resolved),
        Ok(_) => LinkAction::Refuse(format!("Not a file: {}", resolved.display())),
        Err(_) => LinkAction::Refuse(format!("File not found: {}", resolved.display())),
    }
}

/// The URL scheme (`https` in `https://x`), if `url` starts with `alpha *( alpha / digit / + / - / . ) ":"`.
fn scheme_of(url: &str) -> Option<&str> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    // A single letter before ':' would be a Windows drive; treat it as a scheme anyway (refused).
    Some(scheme)
}

/// A `mailto:` link may only carry `subject`, `body`, `cc`, `bcc` and `to` query fields:
/// some mail clients honour `attach=` and would attach a local file.
fn mailto_query_ok(url: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else { return true };
    query.split('&').all(|kv| {
        let key = kv.split('=').next().unwrap_or("");
        let key = crate::cli::percent_decode(key).unwrap_or_else(|| key.to_string());
        matches!(key.trim().to_ascii_lowercase().as_str(), "subject" | "body" | "cc" | "bcc" | "to" | "")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailto_attach_refused() {
        assert!(mailto_query_ok("mailto:a@b.c"));
        assert!(mailto_query_ok("mailto:a@b.c?subject=hi&body=x&cc=d@e.f"));
        assert!(!mailto_query_ok("mailto:a@b.c?attach=/home/u/.ssh/id_rsa"));
        assert!(!mailto_query_ok("mailto:a@b.c?subject=x&ATTACH=/etc/passwd"));
        assert!(!mailto_query_ok("mailto:a@b.c?%61ttach=/etc/passwd"));
        assert!(!mailto_query_ok("mailto:a@b.c?attachment=/x"));
    }

    #[test]
    fn web_and_mail_links_are_external() {
        assert_eq!(classify("https://example.com/a?b#c", None), LinkAction::External("https://example.com/a?b#c".into()));
        assert_eq!(classify("HTTP://example.com", None), LinkAction::External("HTTP://example.com".into()));
        assert_eq!(classify("  <mailto:a@b.c>  ", None), LinkAction::External("mailto:a@b.c".into()));
        assert!(matches!(classify("http:foo", None), LinkAction::Refuse(_)));
    }

    #[test]
    fn dangerous_schemes_are_refused() {
        for u in ["javascript:alert(1)", "JavaScript:alert(1)", "file:///usr/bin/bash", "data:text/html,x", "vbscript:x", "ftp://x", "c:\\x.md", "smb://host/x.md"] {
            assert!(matches!(classify(u, Some(Path::new("/tmp"))), LinkAction::Refuse(_)), "{u}");
        }
        assert!(matches!(classify("https://x\n", None), LinkAction::External(_)));
        assert!(matches!(classify("https://x\u{7}y", None), LinkAction::Refuse(_)));
    }

    #[test]
    fn relative_markdown_links() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("other doc.md"), "x").unwrap();
        std::fs::write(dir.path().join("script.sh"), "rm -rf /").unwrap();
        std::fs::create_dir(dir.path().join("sub.md")).unwrap();
        let base = Some(dir.path());
        assert_eq!(classify("other%20doc.md#intro", base), LinkAction::OpenDoc(dir.path().join("other doc.md")));
        assert_eq!(classify("./other doc.md", base), LinkAction::OpenDoc(dir.path().join("./other doc.md")));
        assert!(matches!(classify("missing.md", base), LinkAction::Refuse(_)));
        assert!(matches!(classify("script.sh", base), LinkAction::Refuse(_)));
        assert!(matches!(classify("sub.md", base), LinkAction::Refuse(_)));
        assert!(matches!(classify("other doc.md", None), LinkAction::Refuse(_)));
        assert!(matches!(classify("", base), LinkAction::Refuse(_)));
        assert!(matches!(classify("?x", base), LinkAction::Refuse(_)));
    }

    #[test]
    fn anchors_and_slugs() {
        assert_eq!(classify("#my-section", None), LinkAction::Anchor("my-section".into()));
        assert_eq!(slugify("Hello, World! 2"), "hello-world-2");
        assert_eq!(slugify("  Café_au-lait "), "café_au-lait");
    }
}
