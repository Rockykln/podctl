use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use podctl::exitcode;

const SYSTEMD_UNIT: &str = include_str!("../../dist/podctld.service");
const TRAY_UNIT: &str = include_str!("../../dist/podctl-tray.service");
const POPUP_UNIT: &str = include_str!("../../dist/podctl-popup.service");
const MAN_PODS: &str = include_str!("../../dist/podctl.1");
const MAN_PODSD: &str = include_str!("../../dist/podctld.1");

pub fn install(args: &[String]) -> i32 {
    let yes = args.iter().any(|a| a == "-y" || a == "--yes");
    let no_daemon = args.iter().any(|a| a == "--no-daemon");
    let no_completion = args.iter().any(|a| a == "--no-completion");
    let no_manpages = args.iter().any(|a| a == "--no-manpages");
    let no_tray = args.iter().any(|a| a == "--no-tray");
    let tray_flag = args.iter().any(|a| a == "--with-tray");
    let no_popup = args.iter().any(|a| a == "--no-popup");
    let popup_flag = args.iter().any(|a| a == "--with-popup");

    let Ok(cur_pods) = std::env::current_exe() else {
        eprintln!("podctl: cannot resolve current binary path");
        return exitcode::OSERR;
    };
    let cur_podctld = cur_pods
        .parent()
        .map(|p| p.join("podctld"))
        .unwrap_or_default();
    if !cur_podctld.exists() {
        eprintln!(
            "podctl: podctld binary not next to podctl at {}",
            cur_podctld.display()
        );
        eprintln!("hint: build with 'cargo build --release' first.");
        return exitcode::UNAVAILABLE;
    }
    let cur_pods_tray = cur_pods
        .parent()
        .map(|p| p.join("podctl-tray"))
        .unwrap_or_default();
    if tray_flag && !cur_pods_tray.exists() {
        eprintln!(
            "podctl: podctl-tray binary not next to podctl at {}",
            cur_pods_tray.display()
        );
        eprintln!("hint: build with 'cargo build --release' first.");
        return exitcode::UNAVAILABLE;
    }
    let cur_pods_popup = cur_pods
        .parent()
        .map(|p| p.join("podctl-popup"))
        .unwrap_or_default();
    if popup_flag && !cur_pods_popup.exists() {
        eprintln!(
            "podctl: podctl-popup binary not next to podctl at {}",
            cur_pods_popup.display()
        );
        eprintln!("hint: build with 'cargo build --release' first.");
        return exitcode::UNAVAILABLE;
    }

    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => {
            eprintln!("podctl: $HOME is not set");
            return exitcode::USAGE;
        }
    };
    let bin_dir = home.join(".local").join("bin");
    let pods_dst = bin_dir.join("podctl");
    let podsd_dst = bin_dir.join("podctld");
    let pods_tray_dst = bin_dir.join("podctl-tray");
    let pods_popup_dst = bin_dir.join("podctl-popup");
    let unit_path = home
        .join(".config")
        .join("systemd")
        .join("user")
        .join("podctld.service");
    let tray_unit_path = home
        .join(".config")
        .join("systemd")
        .join("user")
        .join("podctl-tray.service");
    let popup_unit_path = home
        .join(".config")
        .join("systemd")
        .join("user")
        .join("podctl-popup.service");
    let man_dir = home.join(".local").join("share").join("man").join("man1");

    let shell = detect_shell();
    let completion_target = completion_path(&shell, &home);
    let has_systemctl = have_systemctl();

    // Tray and popup are optional UI, so they are offered rather than
    // assumed: ask whenever the binary is at hand and no flag decided.
    // Skipping the popup silently is what left the tray's own left-click
    // — which asks podctl-popup to draw — doing nothing at all.
    let tray_offered = !no_tray && !tray_flag && cur_pods_tray.exists() && has_systemctl;
    let popup_offered = !no_popup && !popup_flag && cur_pods_popup.exists() && has_systemctl;
    let asked = "   (optional — asked below)";
    let tray_note = if tray_offered { asked } else { "" };
    let popup_note = if popup_offered { asked } else { "" };
    let mut with_tray = tray_flag;
    let mut with_popup = popup_flag;

    println!("podctl will install the following:");
    println!();
    println!("  binaries:");
    println!("    {}", pods_dst.display());
    println!("    {}", podsd_dst.display());
    if tray_flag || tray_offered {
        println!("    {}{tray_note}", pods_tray_dst.display());
    }
    if popup_flag || popup_offered {
        println!("    {}{popup_note}", pods_popup_dst.display());
    }
    if !no_completion {
        println!("  shell completion ({shell}):");
        println!("    {}", completion_target.display());
    }
    if !no_manpages {
        println!("  man pages:");
        println!("    {}", man_dir.join("podctl.1").display());
        println!("    {}", man_dir.join("podctld.1").display());
    }
    if !no_daemon {
        println!("  systemd user service:");
        println!("    {}", unit_path.display());
    }
    if tray_flag || tray_offered {
        println!("  systemd user service (tray):");
        println!("    {}{tray_note}", tray_unit_path.display());
    }
    if popup_flag || popup_offered {
        println!("  systemd user service (popup):");
        println!("    {}{popup_note}", popup_unit_path.display());
    }
    println!();
    println!("no root needed; rollback any time with 'podctl uninstall'.");
    println!();

    if !yes && !confirm("Proceed?", true) {
        println!("aborted.");
        return exitcode::OK;
    }

    // Asked up front so a "no" leaves no stray binary behind.
    if tray_offered {
        with_tray = yes || confirm("Install the tray icon (podctl-tray)?", true);
    }
    if popup_offered {
        with_popup = yes || confirm("Install the case-open popup (podctl-popup)?", true);
    }

    if let Err(e) = std::fs::create_dir_all(&bin_dir) {
        eprintln!("podctl: create {}: {e}", bin_dir.display());
        return exitcode::OSERR;
    }
    if let Err(e) = copy_replace(&cur_pods, &pods_dst) {
        eprintln!("podctl: copy podctl: {e}");
        return exitcode::OSERR;
    }
    if let Err(e) = copy_replace(&cur_podctld, &podsd_dst) {
        eprintln!("podctl: copy podctld: {e}");
        return exitcode::OSERR;
    }
    if with_tray && let Err(e) = copy_replace(&cur_pods_tray, &pods_tray_dst) {
        eprintln!("podctl: copy podctl-tray: {e}");
        return exitcode::OSERR;
    }
    if with_popup && let Err(e) = copy_replace(&cur_pods_popup, &pods_popup_dst) {
        eprintln!("podctl: copy podctl-popup: {e}");
        return exitcode::OSERR;
    }
    println!("  installed binaries");

    if !no_completion {
        if let Some(parent) = completion_target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let script = match shell.as_str() {
            "bash" => BASH_COMPLETION,
            "zsh" => ZSH_COMPLETION,
            "fish" => FISH_COMPLETION,
            _ => "",
        };
        if !script.is_empty() {
            if let Err(e) = std::fs::write(&completion_target, script) {
                eprintln!("podctl: write completion: {e}");
            } else {
                println!("  installed completion ({shell})");
            }
        }
    }

    if !no_manpages {
        if let Err(e) = std::fs::create_dir_all(&man_dir) {
            eprintln!("podctl: create {}: {e}", man_dir.display());
        } else {
            let _ = std::fs::write(man_dir.join("podctl.1"), MAN_PODS);
            let _ = std::fs::write(man_dir.join("podctld.1"), MAN_PODSD);
            println!("  installed man pages");
        }
    }

    let mut daemon_enabled = false;
    let mut tray_enabled = false;
    let mut popup_enabled = false;
    if !no_daemon && !has_systemctl {
        println!();
        println!("note: systemctl not found — skipping systemd user service install.");
        println!(
            "      binaries are in {}; run 'podctld' to start the daemon manually,",
            bin_dir.display()
        );
        println!("      or wire it into your init system (openrc, runit, s6, …).");
    }
    if (with_tray || with_popup) && !has_systemctl {
        println!("      same for podctl-tray / podctl-popup — start them by hand.");
    }
    if !no_daemon && has_systemctl {
        let want_daemon =
            yes || confirm("Enable background daemon (battery + watch events)?", true);
        if want_daemon {
            if let Some(parent) = unit_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let unit =
                SYSTEMD_UNIT.replace("/usr/local/bin/podctld", &podsd_dst.display().to_string());
            if let Err(e) = std::fs::write(&unit_path, unit) {
                eprintln!("podctl: write unit: {e}");
            } else {
                let _ = run("systemctl", &["--user", "daemon-reload"]);
                if run("systemctl", &["--user", "enable", "--now", "podctld"]).is_ok() {
                    daemon_enabled = true;
                    println!("  enabled podctld.service");
                } else {
                    eprintln!(
                        "podctl: systemctl --user enable failed (unit written but not started)"
                    );
                }
            }
        } else {
            let marker = home.join(".config").join("podctl").join("no-daemon");
            if let Some(parent) = marker.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&marker, "");
            println!("  daemon skipped (marker written, no banner shown)");
        }
    }

    if with_tray && has_systemctl {
        if let Some(parent) = tray_unit_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let unit = TRAY_UNIT.replace(
            "/usr/local/bin/podctl-tray",
            &pods_tray_dst.display().to_string(),
        );
        if let Err(e) = std::fs::write(&tray_unit_path, unit) {
            eprintln!("podctl: write tray unit: {e}");
        } else {
            let _ = run("systemctl", &["--user", "daemon-reload"]);
            if run("systemctl", &["--user", "enable", "--now", "podctl-tray"]).is_ok() {
                tray_enabled = true;
                println!("  enabled podctl-tray.service");
            } else {
                eprintln!(
                    "podctl: systemctl --user enable podctl-tray failed (unit written but not started)"
                );
            }
        }
    }

    if with_popup && has_systemctl {
        if let Some(parent) = popup_unit_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let unit = POPUP_UNIT.replace(
            "/usr/local/bin/podctl-popup",
            &pods_popup_dst.display().to_string(),
        );
        if let Err(e) = std::fs::write(&popup_unit_path, unit) {
            eprintln!("podctl: write popup unit: {e}");
        } else {
            let _ = run("systemctl", &["--user", "daemon-reload"]);
            if run("systemctl", &["--user", "enable", "--now", "podctl-popup"]).is_ok() {
                popup_enabled = true;
                println!("  enabled podctl-popup.service");
            } else {
                eprintln!(
                    "podctl: systemctl --user enable podctl-popup failed (unit written but not started)"
                );
            }
        }
    }

    println!();
    println!("installed.");
    if !in_path(&bin_dir) {
        println!();
        println!(
            "note: {} is not in $PATH yet. Add to your shell rc:",
            bin_dir.display()
        );
        println!("  bash/zsh:  export PATH=\"$HOME/.local/bin:$PATH\"");
        println!("  fish:      set -Ux fish_user_paths $HOME/.local/bin $fish_user_paths");
    }
    if !no_manpages
        && std::env::var("MANPATH")
            .map(|m| !m.contains(".local/share/man"))
            .unwrap_or(true)
    {
        let manpath_dir = home.join(".local").join("share").join("man");
        if !is_in_default_manpath(&manpath_dir) {
            println!();
            println!("note: 'man podctl' may need MANPATH:");
            println!("  export MANPATH=\"$HOME/.local/share/man:$MANPATH\"");
        }
    }
    if daemon_enabled {
        println!();
        println!("daemon status: systemctl --user status podctld");
    }
    if tray_enabled {
        println!("tray status:   podctl tray status");
    }
    if popup_enabled {
        println!("popup status:  systemctl --user status podctl-popup");
    }
    println!();
    println!("try:  podctl status");
    exitcode::OK
}

pub fn uninstall(args: &[String]) -> i32 {
    let yes = args.iter().any(|a| a == "-y" || a == "--yes");

    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => {
            eprintln!("podctl: $HOME is not set");
            return exitcode::USAGE;
        }
    };

    let xdg = XdgPaths::from_home(&home);

    // A distro package is detected no matter which copy is running, but it
    // is only ours to remove when we *are* that copy: `podctl install`
    // shadows /usr/bin/podctl in $PATH, and the user-level uninstall must
    // not reach into a package it was not asked about.
    let pkg = owning_package();
    let owned = pkg.is_some() && running_from_system_bin();
    let system_pkg = if owned { pkg.clone() } else { None };
    let foreign_pkg = if owned { None } else { pkg };

    if system_pkg.is_none() && !xdg.any_exists() {
        match &foreign_pkg {
            Some(pkg) => {
                println!("podctl: no per-user files to remove.");
                println!("note: system package '{pkg}' is installed and was left alone.");
                println!("      remove it with: sudo pacman -R {pkg}");
            }
            None => {
                println!(
                    "podctl: nothing to remove (neither system package nor user files found)."
                );
            }
        }
        return exitcode::OK;
    }

    println!("podctl uninstall will remove:");
    if let Some(pkg) = &system_pkg {
        println!("  system package via 'sudo pacman -R {pkg}':");
        println!("    /usr/bin/podctl, podctld, podctl-tray, podctl-popup");
        println!("    /usr/share/man/man1/podctl{{,d}}.1");
        println!("    /usr/lib/systemd/user/podctl{{,d}}*.service");
        println!("    /usr/share/bash-completion/completions/podctl  (and zsh / fish equivalents)");
    }
    let user_paths = xdg.existing_paths();
    if !user_paths.is_empty() {
        println!("  per-user files (`podctl install`):");
        for p in &user_paths {
            println!("    {}", p.display());
        }
    }
    if let Some(pkg) = &foreign_pkg {
        println!("  left alone:");
        println!("    system package '{pkg}' — /usr/bin/podctl and its services");
        println!("    remove it with: sudo pacman -R {pkg}");
    }
    println!();

    if !yes && !confirm("Proceed?", true) {
        println!("aborted.");
        return exitcode::OK;
    }

    // Stop services before removing units, otherwise systemd holds a
    // reference to the now-vanished unit file. Only ever touch a unit this
    // run also removes — stopping a packaged service we are leaving in
    // place would silently kill a tray the user never asked us about.
    // Skip silently on non-systemd hosts — nothing to disable there.
    if have_systemctl() {
        for unit in xdg.units_to_disable(system_pkg.is_some()) {
            let _ = run("systemctl", &["--user", "disable", "--now", unit]);
        }
    }

    let mut rc = exitcode::OK;

    if let Some(pkg) = system_pkg {
        rc = pacman_remove(&pkg);
    }

    // Always sweep user files, even on system installs: a user may have
    // run `podctl install` on top, leaving stray binaries / units / docs
    // in ~/.local and ~/.config.
    xdg.purge();

    if have_systemctl() {
        let _ = run("systemctl", &["--user", "daemon-reload"]);
    }

    if rc == exitcode::OK {
        println!("removed.");
    }
    rc
}

struct XdgPaths {
    bin: Vec<PathBuf>,
    unit: Vec<PathBuf>,
    man: Vec<PathBuf>,
    completion: Vec<PathBuf>,
    marker_dir: PathBuf,
}

impl XdgPaths {
    fn from_home(home: &Path) -> Self {
        let bin_dir = home.join(".local").join("bin");
        let unit_dir = home.join(".config").join("systemd").join("user");
        let man_dir = home.join(".local").join("share").join("man").join("man1");
        let marker_dir = home.join(".config").join("podctl");
        Self {
            bin: vec![
                bin_dir.join("podctl"),
                bin_dir.join("podctld"),
                bin_dir.join("podctl-tray"),
                bin_dir.join("podctl-popup"),
            ],
            unit: vec![
                unit_dir.join("podctld.service"),
                unit_dir.join("podctl-tray.service"),
                unit_dir.join("podctl-popup.service"),
            ],
            man: vec![man_dir.join("podctl.1"), man_dir.join("podctld.1")],
            // Every shell's path, not just this shell's: install
            // writes the one for $SHELL, but a user who later
            // uninstalls from a different shell would otherwise leave
            // the first one behind for good.
            completion: ["bash", "zsh", "fish"]
                .iter()
                .map(|sh| completion_path(sh, home))
                .collect(),
            marker_dir,
        }
    }

    fn all(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.bin.to_vec();
        v.extend(self.unit.iter().cloned());
        v.extend(self.man.iter().cloned());
        v.extend(self.completion.iter().cloned());
        v.push(self.marker_dir.join("no-daemon"));
        v
    }

    fn existing_paths(&self) -> Vec<PathBuf> {
        self.all().into_iter().filter(|p| p.exists()).collect()
    }

    fn any_exists(&self) -> bool {
        self.all().iter().any(|p| p.exists())
    }

    /// Service names whose unit files this run removes: the per-user ones
    /// that exist, plus all three when the package itself is going away.
    fn units_to_disable(&self, removing_package: bool) -> Vec<&'static str> {
        const UNITS: [(&str, &str); 3] = [
            ("podctld", "podctld.service"),
            ("podctl-tray", "podctl-tray.service"),
            ("podctl-popup", "podctl-popup.service"),
        ];
        UNITS
            .iter()
            .filter(|(_, file)| {
                removing_package
                    || self.unit.iter().any(|p| {
                        p.file_name().and_then(|n| n.to_str()) == Some(*file) && p.exists()
                    })
            })
            .map(|(name, _)| *name)
            .collect()
    }

    fn purge(&self) {
        for p in self.all() {
            let _ = std::fs::remove_file(&p);
        }
        let _ = std::fs::remove_dir(&self.marker_dir);
    }
}

/// Whether the running binary is the packaged one, and `uninstall` may
/// therefore hand the package to the package manager.
fn running_from_system_bin() -> bool {
    std::env::current_exe()
        .map(|p| p.starts_with("/usr/bin") || p.starts_with("/usr/local/bin"))
        .unwrap_or(false)
}

fn pacman_remove(pkg: &str) -> i32 {
    let status = Command::new("sudo")
        .args(["pacman", "-R", "--noconfirm", pkg])
        .status();
    match status {
        Ok(s) if s.success() => exitcode::OK,
        Ok(_) => {
            eprintln!("podctl: 'sudo pacman -R {pkg}' returned non-zero.");
            exitcode::SOFTWARE
        }
        Err(e) => {
            eprintln!("podctl: could not spawn sudo: {e}");
            eprintln!("hint: run 'sudo pacman -R {pkg}' yourself.");
            exitcode::UNAVAILABLE
        }
    }
}

fn owning_package() -> Option<String> {
    let out = Command::new("pacman")
        .args(["-Qoq", "/usr/bin/podctl"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

fn confirm(prompt: &str, default_yes: bool) -> bool {
    if !stdin_is_tty() {
        eprintln!("podctl: not on a TTY — re-run with --yes to confirm non-interactively.");
        std::process::exit(exitcode::USAGE);
    }
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{prompt} {hint} ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line).is_err() {
        return default_yes;
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "" => default_yes,
        "y" | "yes" | "j" | "ja" => true,
        _ => false,
    }
}

fn stdin_is_tty() -> bool {
    unsafe { libc_isatty(0) != 0 }
}

unsafe extern "C" {
    #[link_name = "isatty"]
    fn libc_isatty(fd: i32) -> i32;
}

fn detect_shell() -> String {
    if let Ok(s) = std::env::var("SHELL")
        && let Some(name) = Path::new(&s).file_name().and_then(|n| n.to_str())
    {
        return name.to_string();
    }
    "bash".into()
}

fn completion_path(shell: &str, home: &Path) -> PathBuf {
    match shell {
        "fish" => home
            .join(".config")
            .join("fish")
            .join("completions")
            .join("podctl.fish"),
        "zsh" => home
            .join(".local")
            .join("share")
            .join("zsh")
            .join("site-functions")
            .join("_pods"),
        _ => home
            .join(".local")
            .join("share")
            .join("bash-completion")
            .join("completions")
            .join("podctl"),
    }
}

fn copy_replace(src: &Path, dst: &Path) -> io::Result<()> {
    use std::io::{Read, Write};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    if same_file(src, dst) {
        std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o755))?;
        return Ok(());
    }
    if dst.exists() {
        std::fs::remove_file(dst)?;
    }
    // Open with the final mode set at create time so the file is never
    // briefly visible under the user's umask before the chmod lands.
    let mut input = std::fs::File::open(src)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o755)
        .open(dst)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n])?;
    }
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
        _ => false,
    }
}

fn in_path(target: &Path) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    path.split(':').any(|p| Path::new(p) == target)
}

fn is_in_default_manpath(dir: &Path) -> bool {
    if let Ok(out) = Command::new("manpath").output() {
        let s = String::from_utf8_lossy(&out.stdout);
        return s.split(':').any(|p| Path::new(p.trim()) == dir);
    }
    false
}

fn run(cmd: &str, args: &[&str]) -> io::Result<()> {
    let status = Command::new(cmd).args(args).status()?;
    if !status.success() {
        return Err(io::Error::other(format!("{cmd} exit {status}")));
    }
    Ok(())
}

pub fn have_systemctl() -> bool {
    Command::new("systemctl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const BASH_COMPLETION: &str = include_str!("../../dist/completion/podctl.bash");
const ZSH_COMPLETION: &str = include_str!("../../dist/completion/_podctl.zsh");
const FISH_COMPLETION: &str = include_str!("../../dist/completion/podctl.fish");

#[cfg(test)]
mod tests {
    use super::XdgPaths;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("podctl-install-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".config/systemd/user")).unwrap();
        dir
    }

    /// A packaged tray/popup kept running once `podctl install` shadowed
    /// /usr/bin/podctl in $PATH: uninstall stopped all three services
    /// while removing only the per-user daemon unit.
    #[test]
    fn only_units_we_remove_get_disabled() {
        let home = scratch("units");
        std::fs::write(home.join(".config/systemd/user/podctld.service"), "").unwrap();
        let xdg = XdgPaths::from_home(&home);

        assert_eq!(xdg.units_to_disable(false), vec!["podctld"]);

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn removing_the_package_disables_all_of_its_units() {
        let home = scratch("pkg");
        let xdg = XdgPaths::from_home(&home);

        assert_eq!(
            xdg.units_to_disable(true),
            vec!["podctld", "podctl-tray", "podctl-popup"]
        );

        let _ = std::fs::remove_dir_all(&home);
    }
}
