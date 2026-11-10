//! Optional Ninimma picker; unsupported installations retain the native dialog.
use super::{FileDialogRequest, save_filters};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn supported(version: &str) -> bool {
    let mut lines = version.trim().lines();
    let Some(version) = lines.next().and_then(|line| line.strip_prefix("ninimma ")) else { return false };
    let parts: Vec<_> = version.split('.').map(str::parse::<u32>).collect();
    matches!(parts.as_slice(), [Ok(_), Ok(_), Ok(_)]) && lines.any(|line| line == "file-dialog-protocol 1")
}
fn executable() -> Option<PathBuf> {
    let name = if cfg!(windows) { "ninimma.exe" } else { "ninimma" };
    let mut dirs: Vec<_> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/bin"));
    }
    dirs.into_iter().map(|dir| dir.join(name)).find(|p| p.is_absolute() && p.is_file())
}
fn output(executable: &Path, args: &[&str], probe: bool) -> Option<Vec<u8>> {
    let mut child = Command::new(executable).args(args).stdin(Stdio::null()).stderr(Stdio::null()).stdout(Stdio::piped()).spawn().ok()?;
    if probe {
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() < Duration::from_secs(2) => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
    }
    let mut bytes = Vec::new();
    let read = child.stdout.take()?.take(65537).read_to_end(&mut bytes);
    if read.is_err() || bytes.len() > 65536 {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    child.wait().ok()?.success().then_some(bytes)
}
fn valid(paths: &[String], request: &FileDialogRequest) -> bool {
    let save = matches!(request, FileDialogRequest::Save { .. });
    paths.len() <= 2000
        && (matches!(request, FileDialogRequest::OpenMany { .. }) || paths.len() <= 1)
        && paths.iter().all(|p| {
            let p = Path::new(p);
            p.is_absolute() && if save { !p.is_dir() && p.parent().is_some_and(Path::is_dir) } else { p.is_file() }
        })
}
pub(super) fn pick(request: &FileDialogRequest) -> Option<Vec<String>> {
    if std::env::var_os("WORDCRAFT_FILE_DIALOG").is_some_and(|v| v == "builtin") {
        return None;
    }
    let executable = executable()?;
    pick_at(&executable, request)
}

fn pick_at(executable: &Path, request: &FileDialogRequest) -> Option<Vec<String>> {
    // A version number alone never promised that --file-dialog was implemented.
    if !supported(std::str::from_utf8(&output(&executable, &["--version"], true)?).ok()?) {
        return None;
    }
    let (save, multiple, name, directory, filters) = match request {
        FileDialogRequest::Open { purpose } | FileDialogRequest::OpenMany { purpose } => (
            false,
            matches!(request, FileDialogRequest::OpenMany { .. }),
            String::new(),
            None,
            wordcraft_ui_egui::open_filters(purpose)
                .iter()
                .map(|(label, exts)| ((*label).to_string(), exts.iter().map(|e| e.to_string()).collect::<Vec<_>>()))
                .collect::<Vec<_>>(),
        ),
        FileDialogRequest::Save { name, dir } => {
            (true, false, name.clone(), dir.clone(), save_filters(name).into_iter().map(|(label, exts)| (label.to_string(), exts)).collect())
        }
    };
    let json = serde_json::json!({"version":1,"save":save,"multiple":multiple,"name":name,"directory":directory,"filters":filters}).to_string();
    let paths: Vec<String> = serde_json::from_slice(&output(&executable, &["--file-dialog", &json], false)?).ok()?;
    valid(&paths, request).then_some(paths)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_old_versions_and_invalid_selections() {
        assert!(!supported("ninimma 0.1.3"));
        assert!(!supported("ninimma 0.1.4\n"));
        assert!(!supported("ninimma 99.0.0\n"));
        assert!(supported("ninimma 0.1.9\nfile-dialog-protocol 1\n"));
        assert!(!supported("ninimma 0.1.9\nfile-dialog-protocol 2\n"));
        assert!(!supported("other 9.0.0"));
        let request = FileDialogRequest::Open { purpose: "model".into() };
        assert!(valid(&[], &request));
        assert!(!valid(&["relative.txt".into()], &request));
        let file = std::env::current_exe().unwrap().to_str().unwrap().to_owned();
        assert!(valid(std::slice::from_ref(&file), &request));
        assert!(!valid(&[file.clone(), file.clone()], &request));
        assert!(valid(&[file.clone(), file], &FileDialogRequest::OpenMany { purpose: "model".into() }));
    }
}

#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    #[test]
    fn process_protocol_preserves_arguments_and_bounds_output() {
        let command = Path::new("/bin/sh");
        let bytes = output(command, &["-c", "printf '%s' \"$1\"", "picker", "[\"/tmp/a file;$(no command)\"]"], false).unwrap();
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), "[\"/tmp/a file;$(no command)\"]");
        assert_eq!(output(command, &["-c", "printf '[]'"], false).unwrap(), b"[]");
        assert!(output(command, &["-c", "exit 1"], false).is_none());
        assert!(output(command, &["-c", "head -c 65537 /dev/zero"], false).is_none());
    }

    #[test]
    fn unadvertised_picker_is_not_launched_and_advertised_cancel_is_final() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!("qanloom-picker-protocol-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let program = directory.join("ninimma");
        let marker = directory.join("unexpected-dialog");
        let request = FileDialogRequest::Open { purpose: "model".into() };
        for (advertisement, expected) in [("", None), ("file-dialog-protocol 1\\n", Some(Vec::<String>::new()))] {
            std::fs::write(&program, format!("#!/bin/sh\nif [ \"$1\" = --version ]; then\n printf 'ninimma 99.0.0\\n{advertisement}'\nelse\n touch \"$(dirname \"$0\")/unexpected-dialog\"\n printf '[]'\nfi\n")).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(pick_at(&program, &request), expected);
            assert_eq!(marker.exists(), expected.is_some());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
