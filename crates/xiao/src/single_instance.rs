use crate::app::AppHandle;
use std::fs::{File, TryLockError};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub(crate) type SecondInstanceHandler = Box<dyn Fn(&AppHandle, Vec<String>, String) + Send + Sync>;

static HANDLER: OnceLock<SecondInstanceHandler> = OnceLock::new();

pub(crate) struct InstanceLock {
    _file: File,
}

pub(crate) enum Role {
    Primary(Option<InstanceLock>),
    Secondary,
}

fn base_name(identifier: &str) -> String {
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
    if user.is_empty() { identifier.to_owned() } else { format!("{identifier}-{user}") }
}

fn base_dir() -> PathBuf {
    let paths = crate::path::PathResolver::new();
    let private = if cfg!(all(unix, not(target_os = "macos"))) { paths.runtime_dir().or_else(|| paths.cache_dir()) } else { None };
    private.unwrap_or_else(std::env::temp_dir)
}

fn server_name(identifier: &str) -> String {
    let name = base_name(identifier);
    if cfg!(windows) {
        return name;
    }
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3));
    base_dir().join(format!("xiao-{hash:016x}.sock")).display().to_string()
}

pub(crate) fn acquire(identifier: &str) -> Role {
    let path = base_dir().join(format!("{}.instance-lock", base_name(identifier)));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let file = match options.open(&path) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("[single-instance] cannot open {}: {e}, running without it", path.display());
            return Role::Primary(None);
        }
    };

    match file.try_lock() {
        Ok(()) => Role::Primary(Some(InstanceLock { _file: file })),
        Err(TryLockError::WouldBlock) => {
            if !hand_over(identifier) {
                eprintln!("[single-instance] the running instance did not take the launch arguments");
            }
            Role::Secondary
        }
        Err(TryLockError::Error(e)) => {
            eprintln!("[single-instance] cannot lock {}: {e}, running without it", path.display());
            Role::Primary(None)
        }
    }
}

fn hand_over(identifier: &str) -> bool {
    let name = server_name(identifier);
    let args: Vec<String> = std::env::args_os().map(|arg| arg.to_string_lossy().into_owned()).collect();
    let cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
    let message = serde_json::json!({ "argv": args, "cwd": cwd }).to_string();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if send(&name, &message).is_ok() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(unix)]
fn send(name: &str, message: &str) -> std::io::Result<()> {
    let mut stream = std::os::unix::net::UnixStream::connect(name)?;
    writeln!(stream, "{message}")?;
    stream.flush()
}

#[cfg(windows)]
fn send(name: &str, message: &str) -> std::io::Result<()> {
    let mut pipe = std::fs::OpenOptions::new().write(true).open(format!(r"\\.\pipe\{name}"))?;
    writeln!(pipe, "{message}")?;
    pipe.flush()
}

pub(crate) fn listen(identifier: &str, lock: InstanceLock, handler: SecondInstanceHandler) -> Option<InstanceLock> {
    let _ = HANDLER.set(handler);
    let name = server_name(identifier);
    if crate::single_instance_listen(&name, on_message) {
        Some(lock)
    } else {
        eprintln!("[single-instance] cannot listen on {name}: later launches will start their own instance");
        None
    }
}

extern "C" fn on_message(line: *const std::os::raw::c_char) {
    if line.is_null() {
        return;
    }
    let line = unsafe { std::ffi::CStr::from_ptr(line) }.to_string_lossy().into_owned();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
        return;
    };

    let argv: Vec<String> = value["argv"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let cwd = value["cwd"].as_str().unwrap_or_default().to_owned();
    crate::app::trace!("[single-instance] second launch: {argv:?}");

    std::thread::spawn(move || {
        let Some(app) = crate::app::wait_for_global_handle(Duration::from_secs(10)) else {
            return;
        };
        crate::deep_link::deliver(Some(&app), crate::deep_link::links_in(argv.get(1..).unwrap_or_default()));
        if let Some(handler) = HANDLER.get() {
            handler(&app, argv, cwd);
        }
    });
}
