use super::{tools, Report, Status};
use std::path::Path;

pub(super) fn wsl_path(path: &str) -> bool {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    path.starts_with("/mnt/")
        || path.starts_with("/home/")
        || path.starts_with("//wsl$/")
        || path.starts_with("//wsl.localhost/")
}

pub(super) fn native(repo: &Path, report: &mut Report) {
    if cfg!(target_os = "macos") {
        report.result(
            "Xcode command line tools",
            tools::probe(repo, "xcode-select", &["-p"]).and_then(|s| {
                if Path::new(&s).is_dir() {
                    Ok(s)
                } else {
                    Err("xcode-select returned a missing developer directory".into())
                }
            }),
        );
        report.result("C compiler", tools::probe(repo, "cc", &["--version"]));
    } else if cfg!(target_os = "linux") {
        report.result("C compiler", tools::probe(repo, "cc", &["--version"]));
        report.result(
            "pkg-config",
            tools::probe(repo, "pkg-config", &["--version"]),
        );
        // Workspace bevy/default_platform enables gilrs -> libudev-sys. The
        // app-world kira/cpal output backend requires alsa-sys. Windowing loads
        // X11/Wayland dynamically; don't falsely require their .pc files.
        for (lib, hint) in [
            (
                "alsa",
                "alsa-lib (Arch), libasound2-dev (Debian/Ubuntu) or alsa-lib-devel (Fedora)",
            ),
            (
                "libudev",
                "systemd-libs (Arch), libudev-dev (Debian/Ubuntu) or systemd-devel (Fedora)",
            ),
        ] {
            report.result(
                &format!("Linux {lib} development library"),
                tools::probe(repo, "pkg-config", &["--modversion", lib])
                    .map(|s| format!("{lib} {s}; pkg-config metadata found, not a link test"))
                    .map_err(|e| format!("{e}; requires {hint}")),
            );
        }
        if std::env::var_os("WSL_DISTRO_NAME").is_some()
            || std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .is_ok_and(|s| s.to_ascii_lowercase().contains("microsoft"))
        {
            report.add("WSL", Status::Warning, "Linux toolchain checks apply inside WSL; a graphical game additionally needs WSLg/display and working GPU/audio drivers, not validated here");
        }
    } else if cfg!(windows) {
        windows(repo, report);
    } else {
        report.add(
            "Native build tools",
            Status::Warning,
            "This platform's compiler, SDK and native libraries are not validated",
        );
    }
}

fn windows(repo: &Path, report: &mut Report) {
    if wsl_path(&repo.to_string_lossy()) {
        report.add("Windows repository path", Status::Blocked,
            "A WSL/Linux checkout path was selected by the Windows Manager; use a native Windows checkout/toolchain or run Manager inside WSL");
    }
    // cl's no-argument invocation intentionally exits nonzero, so presence is
    // diagnostic only. Rust cc can locate VS without a Developer Prompt.
    let cl = tools::discover("cl", repo);
    let link = tools::discover("link", repo);
    if let (Some(cl), Some(link)) = (cl, link) {
        report.add("MSVC tools", Status::Warning, format!("Candidates: {} and {}; identity/linking not validated. Use Visual Studio Build Tools C++ workload (MSVC), not GNU link.exe", cl.display(), link.display()));
    } else {
        report.add("MSVC tools", Status::Blocked,
            "cl.exe/link.exe not both discoverable. Open Manager from a Visual Studio Developer PowerShell/Native Tools terminal, then Recheck. If not installed, install Desktop development with C++ and a Windows SDK. This conservative check requires an initialized developer environment; it does not prove Visual Studio is absent");
    }
    let sdk = std::env::var_os("WindowsSdkDir")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir());
    match sdk {
        Some(path) if path.join("Include").is_dir() && path.join("Lib").is_dir() => {
            report.add("Windows SDK", Status::Warning, format!("{} has Include/Lib directories; exact SDK libraries and linking still require a build", path.display()));
        }
        _ => report.add("Windows SDK", Status::Blocked,
            "No usable WindowsSdkDir with Include/Lib directories. Open a Visual Studio Developer PowerShell with a Windows 10/11 SDK selected, then Recheck; install the SDK if absent"),
    }
    report.add("Windows build qualification", Status::Warning,
        "These are installation diagnostics only, not a compiler/SDK link test; successful Rust versions do not certify a native Windows game build");
}
