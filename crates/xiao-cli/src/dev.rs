use crate::conf::Conf;
use crate::util::*;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant, SystemTime};

const POLL: Duration = Duration::from_millis(400);
const RESTART_EXIT_CODE: i32 = 75;

pub fn run(conf: &Conf) -> Result<(), String> {
    let dev_url = std::env::var("XIAO_DEV_URL").unwrap_or_else(|_| conf.str_at(&["build", "devUrl"]).unwrap_or("http://localhost:1420").to_owned());
    let mut watch = conf.files.clone();
    watch.push(conf.app.clone());
    for key in [["build", "additionalWatchFolders"], ["bundle", "resources"]] {
        watch.extend(conf.strings_at(&key).iter().map(|entry| conf.root.join(entry)));
    }

    log("dev", GREEN, &format!("{} dev", conf.product_name()));

    let mut frontend = start_frontend(conf, &dev_url)?;
    if !wait_for(&dev_url, Duration::from_secs(30)) {
        stop(&mut frontend);
        return Err(format!("dev server did not come up at {dev_url}"));
    }
    log("dev", GREEN, &format!("dev server is up at {dev_url}"));

    let mut app = None;
    if build(conf)? {
        app = start_app(conf, &dev_url);
    }

    log("dev", DIM, "in-window: F5 reload · F12 devtools · Ctrl+C to quit");
    let listed: Vec<String> = watch.iter().map(|dir| dir.strip_prefix(&conf.root).unwrap_or(dir).display().to_string()).collect();
    log("dev", GREEN, &format!("watching {}", listed.join(", ")));

    let mut fingerprint = snapshot(&watch);
    loop {
        std::thread::sleep(POLL);

        if let Some(child) = app.as_mut()
            && let Ok(Some(status)) = child.try_wait()
        {
            if status.code() == Some(RESTART_EXIT_CODE) {
                log("app", BLUE, "restart requested, launching again");
                app = start_app(conf, &dev_url);
                continue;
            }
            log("app", YELLOW, &format!("app exited ({status}), shutting down"));
            stop(&mut app);
            stop(&mut frontend);
            return Ok(());
        }

        if let Some(child) = frontend.as_mut()
            && matches!(child.try_wait(), Ok(Some(_)))
        {
            stop(&mut app);
            stop(&mut frontend);
            return Err("dev server exited".to_owned());
        }

        let current = snapshot(&watch);
        if current == fingerprint {
            continue;
        }
        fingerprint = current;

        log("dev", YELLOW, "sources changed, rebuilding");
        match build(conf) {
            Ok(true) => {
                stop(&mut app);
                app = start_app(conf, &dev_url);
            }
            Ok(false) => log("cargo", RED, "build failed, keeping the old binary running"),
            Err(e) => log("cargo", RED, &e),
        }
    }
}

fn start_frontend(conf: &Conf, dev_url: &str) -> Result<Option<Child>, String> {
    if is_listening(dev_url) {
        log("dev", YELLOW, &format!("something is already serving {dev_url}, reusing it"));
        return Ok(None);
    }

    let Some(command) = conf.str_at(&["build", "beforeDevCommand"]) else {
        log("dev", YELLOW, "no build.beforeDevCommand, assuming the frontend dev server is already running");
        return Ok(None);
    };

    log("vite", MAGENTA, &format!("starting: {command}"));
    let (shell, first) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
    let mut child = std::process::Command::new(shell)
        .args([first, command])
        .current_dir(&conf.root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start beforeDevCommand: {e}"))?;
    track(&child);
    pipe(&mut child, "vite", MAGENTA);
    Ok(Some(child))
}

fn is_listening(url: &str) -> bool {
    let authority = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url).split('/').next().unwrap_or(url);

    let with_port = match authority.rsplit_once(':') {
        Some(_) => authority.to_owned(),
        None => format!("{authority}:{}", if url.starts_with("https") { 443 } else { 80 }),
    };

    match with_port.to_socket_addrs() {
        Ok(addrs) => addrs.into_iter().any(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()),
        Err(_) => false,
    }
}

fn wait_for(url: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if is_listening(url) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn build(conf: &Conf) -> Result<bool, String> {
    log("cargo", CYAN, "building…");
    let started = Instant::now();

    let status = conf.cargo()?.arg("build").status().map_err(|e| format!("cargo: {e}"))?;

    if status.success() {
        stage_resources(conf, &conf.target_dir().join("debug"))?;
        log("cargo", GREEN, &format!("built in {:.1}s", started.elapsed().as_secs_f32()));
    }
    Ok(status.success())
}

fn start_app(conf: &Conf, dev_url: &str) -> Option<Child> {
    let bin = conf.target_dir().join("debug").join(format!("{}{}", conf.binary_name(), std::env::consts::EXE_SUFFIX));

    if !bin.is_file() {
        log("app", RED, &format!("binary not found: {}", bin.display()));
        return None;
    }

    log("app", BLUE, "launching…");
    let mut command = std::process::Command::new(&bin);
    command
        .arg("--dev")
        .current_dir(&conf.root)
        .env("XIAO_DEV_URL", dev_url)
        .env("QT_FORCE_STDERR_LOGGING", "1")
        .env("QT_ASSUME_STDERR_HAS_CONSOLE", "1")
        .env("QTWEBENGINE_REMOTE_DEBUGGING", std::env::var("QTWEBENGINE_REMOTE_DEBUGGING").unwrap_or_else(|_| "9222".to_owned()))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if cfg!(target_os = "linux")
        && let Some(path) = qt_library_path()
    {
        command.env("LD_LIBRARY_PATH", path);
    }
    let mut child = command.spawn().ok()?;

    track(&child);
    pipe(&mut child, "app", BLUE);
    Some(child)
}

fn qt_library_path() -> Option<std::ffi::OsString> {
    let libs = crate::qt::QtPaths::query().ok()?.libs;
    if libs.starts_with("/usr") {
        return None;
    }
    let existing = std::env::var_os("LD_LIBRARY_PATH").unwrap_or_default();
    std::env::join_paths(std::iter::once(libs).chain(std::env::split_paths(&existing))).ok()
}

fn stop(child: &mut Option<Child>) {
    if let Some(running) = child.as_mut() {
        kill_tree(running);
    }
    *child = None;
}

fn snapshot(watch: &[std::path::PathBuf]) -> Option<SystemTime> {
    let mut newest = None;

    for dir in watch {
        walk(dir, &mut |path| {
            if let Ok(modified) = path.metadata().and_then(|m| m.modified()) {
                newest = newest.max(Some(modified));
            }
        });
    }
    newest
}

fn walk(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        if dir.is_file() {
            visit(dir);
        }
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if name == "target" || name.starts_with('.') || name.ends_with('~') {
            continue;
        }
        if path.is_dir() {
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
}
