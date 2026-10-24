//! Showing files in the OS file manager (`Media::Reveal`, desktop).
//!
//! - macOS: `open -R <file>` (Finder, the file selected); a folder to open: `open <dir>`.
//! - Windows: `explorer /select,"<file>"`; a folder: `explorer <dir>`.
//! - Linux and the other Unixes: the freedesktop file manager's `ShowItems` D-Bus call
//!   (`dbus-send`, the file selected: Nautilus, Dolphin, Nemo, Thunar, ...), falling back to
//!   `xdg-open` on the parent folder when no file manager answers; a folder: `xdg-open <dir>`.
//!
//! The launchers run on a thread of their own, so a slow D-Bus answer never holds up the
//! controller; their failures are only logged (the file exists, checked first).

use std::ffi::OsString;
use std::path::Path;

use ether_controller::store::StoreError;

/// One launcher to try. With `check`, a failing exit status moves on to the next one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub program: &'static str,
    pub args: Vec<OsString>,
    pub check: bool,
}

/// Show `path` in the file manager: selected in its folder, or with `open`, opened.
/// `NotFound` when nothing is there.
pub fn reveal(path: &str, open: bool) -> Result<(), StoreError> {
    let p = Path::new(path);
    if path.contains('\0') || !p.is_absolute() {
        return Err(StoreError::InvalidPath(path.to_string()));
    }
    let meta = std::fs::metadata(p).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => StoreError::NotFound(path.to_string()),
        _ => StoreError::Io(e.to_string()),
    })?;
    let steps = steps(p, open && meta.is_dir());
    std::thread::Builder::new()
        .name("ether-reveal".into())
        .spawn(move || run(&steps))
        .map(drop)
        .map_err(|e| StoreError::Io(e.to_string()))
}

fn run(steps: &[Step]) {
    for step in steps {
        let mut c = std::process::Command::new(step.program);
        add_args(&mut c, &step.args);
        c.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        match c.status() {
            Ok(s) if s.success() || !step.check => return,
            Ok(s) => tracing::debug!(program = step.program, %s, "reveal: trying the next way"),
            Err(e) => tracing::debug!(program = step.program, %e, "reveal: trying the next way"),
        }
    }
    tracing::warn!("could not open the file manager");
}

#[cfg(windows)]
fn add_args(c: &mut std::process::Command, args: &[OsString]) {
    // `explorer` parses its own command line: `/select,"C:\a b\c.wav"` must reach it as is.
    use std::os::windows::process::CommandExt;
    for a in args {
        c.raw_arg(a);
    }
}

#[cfg(not(windows))]
fn add_args(c: &mut std::process::Command, args: &[OsString]) {
    c.args(args);
}

/// The launchers for this OS (see the module docs).
#[cfg(target_os = "macos")]
pub fn steps(path: &Path, open: bool) -> Vec<Step> {
    let mut args: Vec<OsString> = Vec::new();
    if !open {
        args.push("-R".into());
    }
    args.push(path.as_os_str().to_owned());
    vec![Step {
        program: "open",
        args,
        check: false,
    }]
}

/// The launchers for this OS (see the module docs).
#[cfg(windows)]
pub fn steps(path: &Path, open: bool) -> Vec<Step> {
    let mut arg = OsString::from(if open { "\"" } else { "/select,\"" });
    arg.push(path.as_os_str());
    arg.push("\"");
    // explorer's exit status is 1 even when it worked.
    vec![Step {
        program: "explorer",
        args: vec![arg],
        check: false,
    }]
}

/// The launchers for this OS (see the module docs).
#[cfg(all(unix, not(target_os = "macos")))]
pub fn steps(path: &Path, open: bool) -> Vec<Step> {
    let xdg_open = |dir: &Path| Step {
        program: "xdg-open",
        args: vec![dir.as_os_str().to_owned()],
        check: false,
    };
    if open {
        return vec![xdg_open(path)];
    }
    vec![
        Step {
            program: "dbus-send",
            args: [
                "--session",
                "--print-reply",
                "--reply-timeout=3000",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
            ]
            .into_iter()
            .map(OsString::from)
            .chain([
                OsString::from(format!("array:string:{}", file_uri(path))),
                OsString::from("string:"),
            ])
            .collect(),
            check: true,
        },
        xdg_open(path.parent().unwrap_or(path)),
    ]
}

/// `file://` URI of an absolute path, every byte outside the unreserved set and `/`
/// percent-encoded (so a `,`, which separates `dbus-send` array items, never splits it).
pub fn file_uri(path: &Path) -> String {
    let bytes = path_bytes(path);
    let mut out = String::from("file://");
    for &b in bytes.iter() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().replace('\\', "/").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_escape_what_a_uri_or_dbus_send_would_misread() {
        assert_eq!(
            file_uri(Path::new("/home/me/Samples/kick, hard #1 é.wav")),
            "file:///home/me/Samples/kick%2C%20hard%20%231%20%C3%A9.wav"
        );
        assert_eq!(file_uri(Path::new("/a/b_c-d.~e")), "file:///a/b_c-d.~e");
    }

    #[test]
    fn missing_or_relative_paths_are_refused() {
        assert!(matches!(
            reveal("relative/file.wav", false),
            Err(StoreError::InvalidPath(_))
        ));
        let gone = std::env::temp_dir().join(format!("ether-reveal-gone-{}", std::process::id()));
        assert!(matches!(
            reveal(gone.to_str().unwrap(), false),
            Err(StoreError::NotFound(_))
        ));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_selects_through_dbus_then_falls_back_to_the_parent_folder() {
        let s = steps(Path::new("/x/y/kick.wav"), false);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].program, "dbus-send");
        assert!(s[0].check);
        assert!(
            s[0].args
                .contains(&OsString::from("array:string:file:///x/y/kick.wav"))
        );
        assert_eq!(s[1].program, "xdg-open");
        assert_eq!(s[1].args, vec![OsString::from("/x/y")]);
        let open = steps(Path::new("/x/projects"), true);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].args, vec![OsString::from("/x/projects")]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_selects_with_open_r() {
        let s = steps(Path::new("/x/kick.wav"), false);
        assert_eq!(s[0].program, "open");
        assert_eq!(
            s[0].args,
            vec![OsString::from("-R"), OsString::from("/x/kick.wav")]
        );
        assert_eq!(
            steps(Path::new("/x"), true)[0].args,
            vec![OsString::from("/x")]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_selects_with_explorer() {
        let s = steps(Path::new(r"C:\x\kick.wav"), false);
        assert_eq!(s[0].program, "explorer");
        assert_eq!(
            s[0].args,
            vec![OsString::from(r#"/select,"C:\x\kick.wav""#)]
        );
    }
}
