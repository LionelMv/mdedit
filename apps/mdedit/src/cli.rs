//! Command-line parsing: `mdedit [OPTIONS] [FILE|DIR]...`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const HELP: &str = "\
mdedit - a native Markdown editor

USAGE:
    mdedit [OPTIONS] [FILE|DIR]...

ARGS:
    FILE    Markdown file to open in a tab. A path that does not exist yet opens
            an empty document that will be saved there.
    DIR     Folder to show in the sidebar file tree.

OPTIONS:
    -h, --help       Print this help and exit
    -V, --version    Print the version and exit
    --               Treat every following argument as a path

ENVIRONMENT:
    RUST_LOG         Log filter, e.g. RUST_LOG=mdedit=debug
";

#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    Help,
    Version,
    Run(Vec<PathBuf>),
}

/// Parse the arguments *after* the program name.
pub fn parse_args<I>(args: I) -> Result<Cli, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut paths = Vec::new();
    let mut only_paths = false;
    for arg in args {
        if !only_paths && let Some(s) = arg.to_str() {
            match s {
                "-h" | "--help" => return Ok(Cli::Help),
                "-V" | "--version" => return Ok(Cli::Version),
                "--" => {
                    only_paths = true;
                    continue;
                }
                _ if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown option '{s}' (see --help)")),
                _ => {}
            }
        }
        if arg.is_empty() {
            continue;
        }
        paths.push(arg_to_path(arg));
    }
    Ok(Cli::Run(paths))
}

/// Accept `file://` URIs (desktop launchers pass `%U`) as well as plain paths.
fn arg_to_path(arg: OsString) -> PathBuf {
    if let Some(s) = arg.to_str()
        && let Some(rest) = s.strip_prefix("file://")
    {
        // file:///abs/path or file://localhost/abs/path
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        if rest.starts_with('/')
            && let Some(decoded) = percent_decode(rest)
        {
            return PathBuf::from(decoded);
        }
    }
    PathBuf::from(arg)
}

/// Decode `%XX` escapes; `None` if the result is not UTF-8 or an escape is malformed.
pub fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// What to open at startup.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StartupTargets {
    /// Files to open in tabs (may not exist yet).
    pub files: Vec<PathBuf>,
    /// The folder for the sidebar (last one given wins).
    pub folder: Option<PathBuf>,
}

/// Split CLI paths into files and folders by looking at the file system.
pub fn classify(paths: &[PathBuf]) -> StartupTargets {
    let mut t = StartupTargets::default();
    for p in paths {
        let p = absolutize(p);
        if p.is_dir() {
            t.folder = Some(p);
        } else {
            t.files.push(p);
        }
    }
    t
}

/// Make `p` absolute against the current dir (without resolving symlinks or requiring it to exist).
pub fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse_args(args(&["--help"])), Ok(Cli::Help));
        assert_eq!(parse_args(args(&["-h", "x.md"])), Ok(Cli::Help));
        assert_eq!(parse_args(args(&["--version"])), Ok(Cli::Version));
        assert_eq!(parse_args(args(&["-V"])), Ok(Cli::Version));
    }

    #[test]
    fn paths_and_double_dash() {
        assert_eq!(parse_args(args(&[])), Ok(Cli::Run(vec![])));
        assert_eq!(parse_args(args(&["a.md", "dir"])), Ok(Cli::Run(vec!["a.md".into(), "dir".into()])));
        assert_eq!(parse_args(args(&["--", "--help", "-x"])), Ok(Cli::Run(vec!["--help".into(), "-x".into()])));
        // A lone "-" is a path, empty args are skipped.
        assert_eq!(parse_args(args(&["-", ""])), Ok(Cli::Run(vec!["-".into()])));
    }

    #[test]
    fn unknown_option_is_error() {
        assert!(parse_args(args(&["--frobnicate"])).is_err());
        assert!(parse_args(args(&["-x", "a.md"])).is_err());
    }

    #[test]
    fn file_uris() {
        assert_eq!(parse_args(args(&["file:///tmp/my%20notes.md"])), Ok(Cli::Run(vec!["/tmp/my notes.md".into()])));
        assert_eq!(parse_args(args(&["file://localhost/tmp/a.md"])), Ok(Cli::Run(vec!["/tmp/a.md".into()])));
        // Malformed escape: kept verbatim.
        assert_eq!(parse_args(args(&["file:///tmp/%zz"])), Ok(Cli::Run(vec!["file:///tmp/%zz".into()])));
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%2Fb%C3%A9").as_deref(), Some("a/bé"));
        assert_eq!(percent_decode("%"), None);
        assert_eq!(percent_decode("%ff"), None); // invalid UTF-8
    }

    #[test]
    fn classify_dirs_and_files() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.md");
        std::fs::write(&f, "# a").unwrap();
        let missing = dir.path().join("new.md");
        let t = classify(&[f.clone(), dir.path().to_path_buf(), missing.clone()]);
        assert_eq!(t.files, vec![f, missing]);
        assert_eq!(t.folder.as_deref(), Some(dir.path()));
    }
}
