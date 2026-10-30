//! Open files in the running window: a later `mdedit` hands its paths to the first one over a
//! private Unix socket and exits.

use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);
const MAX_MESSAGE: u64 = 1024 * 1024;

/// Outcome of [`claim`].
pub enum Claim {
    /// A running window took the paths: this process should exit.
    Forwarded,
    /// We are the first: listen on this socket.
    First(UnixListener),
}

/// Hand `paths` to a running window, or take over the socket at `sock`.
pub fn claim(sock: &Path, paths: &[PathBuf]) -> io::Result<Claim> {
    if send(sock, paths).is_ok() {
        return Ok(Claim::Forwarded);
    }
    if let Some(dir) = sock.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = match UnixListener::bind(sock) {
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            // Another start may have just won the race; otherwise the socket file is stale.
            if send(sock, paths).is_ok() {
                return Ok(Claim::Forwarded);
            }
            std::fs::remove_file(sock)?;
            UnixListener::bind(sock)?
        }
        other => other?,
    };
    std::fs::set_permissions(sock, std::fs::Permissions::from_mode(0o600))?;
    Ok(Claim::First(listener))
}

/// Call `on_open` from a background thread with the paths each later start sends. An empty list
/// means it was started without files.
pub fn listen(listener: UnixListener, on_open: impl Fn(Vec<PathBuf>) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream.and_then(|s| read_paths(&s)) {
                Ok(paths) => on_open(paths),
                Err(e) => log::warn!("open request: {e}"),
            }
        }
    });
}

fn send(sock: &Path, paths: &[PathBuf]) -> io::Result<()> {
    let mut stream = UnixStream::connect(sock)?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    // NUL-separated: the one byte a path cannot contain.
    for p in paths {
        stream.write_all(p.as_os_str().as_bytes())?;
        stream.write_all(&[0])?;
    }
    Ok(())
}

fn read_paths(stream: &UnixStream) -> io::Result<Vec<PathBuf>> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    let mut bytes = Vec::new();
    stream.take(MAX_MESSAGE).read_to_end(&mut bytes)?;
    Ok(bytes.split(|&b| b == 0).filter(|p| !p.is_empty()).map(|p| std::ffi::OsString::from_vec(p.to_vec()).into()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn second_start_forwards_paths_to_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("run/mdedit.sock");
        let Claim::First(listener) = claim(&sock, &[]).unwrap() else { panic!("nobody is listening yet") };
        let (tx, rx) = mpsc::channel();
        listen(listener, move |paths| tx.send(paths).unwrap());
        let paths = vec![PathBuf::from("/a b/é.md"), PathBuf::from("/line\nbreak.md")];
        assert!(matches!(claim(&sock, &paths).unwrap(), Claim::Forwarded));
        assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), paths);
        assert!(matches!(claim(&sock, &[]).unwrap(), Claim::Forwarded));
        assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), Vec::<PathBuf>::new());
        assert_eq!(std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn stale_socket_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("mdedit.sock");
        drop(UnixListener::bind(&sock).unwrap()); // leaves the file behind, like a crash
        assert!(sock.exists());
        assert!(matches!(claim(&sock, &[]).unwrap(), Claim::First(_)));
    }
}
