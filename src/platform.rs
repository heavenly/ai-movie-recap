use std::path::Path;

pub fn open_folder<P: AsRef<Path>>(path: P) {
    let path = path.as_ref();
    if path.as_os_str().is_empty() {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}
