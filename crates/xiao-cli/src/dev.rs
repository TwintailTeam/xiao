use crate::conf::{CONFIG_FILE, Conf};
use crate::util::*;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant, SystemTime};

const POLL: Duration = Duration::from_millis(400);
const RESTART_EXIT_CODE: i32 = 75;

pub fn run(conf: &Conf) -> Result<(), String> {
    let dev_url = conf.dev_url();
    let mut watch = vec![conf.app.clone()];
    watch.extend(conf.strings_at(&["build", "additionalWatchFolders"]).iter().map(|dir| conf.root.join(dir)));
    watch.extend(conf.strings_at(&["bundle", "resources"]).iter().map(|entry| conf.asset(entry)));

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

    let mut fingerprint = snapshot(conf, &watch);
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

        let current = snapshot(conf, &watch);
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
    let mut child = spawn_piped(shell, &[first, command], &conf.root).map_err(|e| format!("cannot start beforeDevCommand: {e}"))?;
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

    let status =
        std::process::Command::new("cargo").args(["build"]).current_dir(&conf.app).envs(crate::qt::cargo_env()?).status().map_err(|e| format!("cargo: {e}"))?;

    if status.success() {
        stage_resources(conf, &conf.target_dir().join("debug"))?;
        log("cargo", GREEN, &format!("built in {:.1}s", started.elapsed().as_secs_f32()));
    }
    Ok(status.success())
}

fn start_app(conf: &Conf, dev_url: &str) -> Option<Child> {
    let bin = conf.target_dir().join("debug").join(exe_name(conf.binary_name()));

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

fn exe_name(name: &str) -> String { if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() } }

fn snapshot(conf: &Conf, watch: &[std::path::PathBuf]) -> Option<SystemTime> {
    let mut newest = conf.root.join(CONFIG_FILE).metadata().and_then(|m| m.modified()).ok();

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
