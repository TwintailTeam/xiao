use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const MAGENTA: &str = "\x1b[35m";
pub const CYAN: &str = "\x1b[36m";

fn use_color() -> bool { std::env::var_os("NO_COLOR").is_none() }

pub fn log(tag: &str, color: &str, message: &str) {
    let label = format!("{tag:<6}");
    let label = if use_color() { format!("{color}{label}{RESET}") } else { label };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{label} {message}");
    let _ = out.flush();
}

pub fn pipe(child: &mut Child, tag: &'static str, color: &'static str) {
    let streams: Vec<Box<dyn Read + Send>> = vec![Box::new(child.stdout.take().expect("piped stdout")), Box::new(child.stderr.take().expect("piped stderr"))];

    for stream in streams {
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let clean = line.strip_prefix("qml: ").unwrap_or(&line);
                if !clean.trim().is_empty() {
                    log(tag, color, clean);
                }
            }
        });
    }
}

pub fn spawn_piped(program: &str, args: &[&str], cwd: &Path) -> std::io::Result<Child> {
    let child = Command::new(program).args(args).current_dir(cwd).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    track(&child);
    Ok(child)
}

pub fn track(child: &Child) {
    #[cfg(windows)]
    job::track(child);
    #[cfg(not(windows))]
    let _ = child;
}

#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::sync::Mutex;

    type Handle = *mut std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const std::ffi::c_void, name: *const u16) -> Handle;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    static JOBS: Mutex<Vec<(u32, usize)>> = Mutex::new(Vec::new());

    pub fn track(child: &Child) {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return;
            }
            if AssignProcessToJobObject(job, child.as_raw_handle()) == 0 {
                CloseHandle(job);
                return;
            }
            JOBS.lock().unwrap().push((child.id(), job as usize));
        }
    }

    pub fn terminate(child: &Child) {
        let mut jobs = JOBS.lock().unwrap();
        let Some(at) = jobs.iter().position(|(pid, _)| *pid == child.id()) else { return };
        let (_, job) = jobs.remove(at);
        unsafe {
            TerminateJobObject(job as Handle, 1);
            CloseHandle(job as Handle);
        }
    }
}

pub fn exec(program: &str, args: &[&str], cwd: &Path) -> Result<(), String> {
    let status = Command::new(program).args(args).current_dir(cwd).status().map_err(|e| format!("{program}: {e}"))?;

    if status.success() { Ok(()) } else { Err(format!("{program} exited with {status}")) }
}

pub fn run_shell(command: &str, cwd: &Path) -> Result<(), String> {
    if cfg!(windows) { exec("cmd", &["/C", command], cwd) } else { exec("sh", &["-c", command], cwd) }
}

pub fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    job::terminate(child);

    #[cfg(unix)]
    {
        let pid = child.id();
        let mut tree = vec![pid];
        collect_descendants(pid, &mut tree);

        signal(&tree, "TERM");
        std::thread::sleep(std::time::Duration::from_millis(300));
        signal(&tree, "KILL");
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn collect_descendants(pid: u32, found: &mut Vec<u32>) {
    for child in children_of(pid) {
        if found.contains(&child) {
            continue;
        }
        found.push(child);
        collect_descendants(child, found);
    }
}

#[cfg(unix)]
fn children_of(pid: u32) -> Vec<u32> {
    let tasks = std::path::PathBuf::from(format!("/proc/{pid}/task"));
    if let Ok(entries) = std::fs::read_dir(&tasks) {
        let listed: Vec<u32> = entries
            .flatten()
            .filter_map(|task| std::fs::read_to_string(task.path().join("children")).ok())
            .flat_map(|line| line.split_ascii_whitespace().filter_map(|pid| pid.parse().ok()).collect::<Vec<u32>>())
            .collect();
        if !listed.is_empty() {
            return listed;
        }
    }
    Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).split_ascii_whitespace().filter_map(|pid| pid.parse().ok()).collect())
        .unwrap_or_default()
}

#[cfg(unix)]
fn signal(pids: &[u32], name: &str) {
    let list = pids.iter().map(u32::to_string).collect::<Vec<_>>().join(" ");
    let _ = Command::new("sh").args(["-c", &format!("kill -{name} {list} 2>/dev/null")]).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

pub fn which(program: &str) -> bool {
    let exts: Vec<String> =
        if cfg!(windows) { std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE".into()).split(';').map(str::to_owned).collect() } else { vec![String::new()] };
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| exts.iter().any(|ext| dir.join(format!("{program}{ext}")).is_file())))
        .unwrap_or(false)
}

pub fn stage_resources(conf: &crate::conf::Conf, dest: &Path) -> Result<(), String> {
    for entry in conf.strings_at(&["bundle", "resources"]) {
        let src = conf.asset(&entry);
        let relative = resource_relpath(Path::new(&entry));
        if relative.as_os_str().is_empty() {
            log("warn", YELLOW, &format!("resource `{entry}` names no file or folder, skipped"));
            continue;
        }
        if !src.exists() {
            log("warn", YELLOW, &format!("resource `{entry}` does not exist, skipped"));
            continue;
        }

        let target = dest.join(&relative);
        if src.is_dir() {
            let _ = std::fs::remove_dir_all(&target);
            copy_dir(&src, &target)?;
        } else {
            copy_file(&src, &target)?;
        }
    }
    Ok(())
}

fn resource_relpath(entry: &Path) -> std::path::PathBuf {
    entry
        .components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name.to_os_string()),
            std::path::Component::ParentDir => Some("_up_".into()),
            std::path::Component::RootDir => Some("_root_".into()),
            std::path::Component::CurDir | std::path::Component::Prefix(_) => None,
        })
        .collect()
}

pub fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;

    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries.flatten() {
        let src = entry.path();
        let dest = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir(&src, &dest)?;
        } else {
            copy_file(&src, &dest)?;
        }
    }
    Ok(())
}

pub fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::copy(from, to).map(|_| ()).map_err(|e| format!("copying {} → {}: {e}", from.display(), to.display()))
}

#[cfg(unix)]
pub fn symlink(target: &str, link: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(target, link).map_err(|e| format!("symlink {} → {target}: {e}", link.display()))
}

#[cfg(not(unix))]
pub fn symlink(_target: &str, _link: &Path) -> Result<(), String> { Err("symlinks are only used by the macOS .app layout".to_owned()) }

#[cfg(unix)]
pub fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("chmod {}: {e}", path.display()))
}

#[cfg(not(unix))]
pub fn make_executable(_path: &Path) -> Result<(), String> { Ok(()) }

pub fn dir_size_kib(dir: &Path) -> u64 {
    fn walk(dir: &Path, total: &mut u64) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, total);
            } else if let Ok(meta) = entry.metadata() {
                *total += meta.len().div_ceil(1024);
            }
        }
    }

    let mut total = 0;
    walk(dir, &mut total);
    total
}

pub fn reset_dir(dir: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

pub fn write(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, contents).map_err(|e| format!("writing {}: {e}", path.display()))
}
