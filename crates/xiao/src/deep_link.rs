use crate::app::{AppHandle, EventId};
use std::sync::Mutex;

pub(crate) const EVENT: &str = "deep-link://new-url";

const RESERVED: &[&str] = &["http", "https", "file", "ftp", "mailto", "data", "javascript", "about", "blob", "xiao", "qrc"];

static CURRENT: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// URL schemes that open the app, from [`AppHandle::deep_link`](crate::AppHandle::deep_link).
///
/// Schemes in `app.deepLink.schemes` are registered by the app's packages.
/// Links that arrive while the app runs come through single instance, so
/// enable [`Shell::single_instance`](crate::Shell::single_instance) as well.
pub struct DeepLinkHandle<'a> {
    pub(crate) app: &'a AppHandle,
}

impl DeepLinkHandle<'_> {
    /// The links the app was most recently opened with.
    pub fn current(&self) -> Vec<String> { CURRENT.lock().unwrap().clone() }

    /// Calls `handler` with the links of each later launch, emitted as `deep-link://new-url`.
    pub fn on_open_url<F>(&self, handler: F) -> EventId
    where F: Fn(&AppHandle, Vec<String>) + Send + Sync + 'static {
        self.app.listen(EVENT, move |app, payload| {
            let urls = payload["urls"].as_array().map(|urls| urls.iter().filter_map(|url| url.as_str().map(str::to_owned)).collect()).unwrap_or_default();
            handler(app, urls);
        })
    }

    /// Makes the app the current user's handler for `scheme`. Unsupported on
    /// macOS, where schemes come only from `Info.plist`.
    pub fn register(&self, scheme: &str) -> Result<(), String> { register(&valid(scheme)?) }

    pub fn unregister(&self, scheme: &str) -> Result<(), String> { unregister(&valid(scheme)?) }

    /// Whether this executable, or this AppImage, is the registered handler for `scheme`.
    pub fn is_registered(&self, scheme: &str) -> bool { valid(scheme).is_ok_and(|scheme| is_registered(&scheme)) }
}

fn valid(scheme: &str) -> Result<String, String> {
    let scheme = scheme.trim().trim_end_matches("://").trim_end_matches(':').to_ascii_lowercase();
    let well_formed = scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !well_formed {
        return Err(format!("`{scheme}` is not a URL scheme"));
    }
    if RESERVED.contains(&scheme.as_str()) {
        return Err(format!("the `{scheme}` scheme cannot be taken over"));
    }
    Ok(scheme)
}

pub(crate) fn configured_schemes() -> Vec<String> {
    crate::config::config()["app"]["deepLink"]["schemes"]
        .as_array()
        .map(|schemes| schemes.iter().filter_map(|scheme| scheme.as_str()).filter_map(|scheme| valid(scheme).ok()).collect())
        .unwrap_or_default()
}

pub(crate) fn links_in(args: &[String]) -> Vec<String> {
    let configured = configured_schemes();
    args.iter()
        .filter(|arg| arg.split_once(':').is_some_and(|(scheme, _)| valid(scheme).is_ok_and(|scheme| configured.contains(&scheme) || is_registered(&scheme))))
        .cloned()
        .collect()
}

pub(crate) fn start() {
    let args: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    *CURRENT.lock().unwrap() = links_in(&args);

    let self_registers = cfg!(windows) || (cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some());
    if self_registers {
        for scheme in configured_schemes() {
            if let Err(e) = register(&scheme) {
                eprintln!("[deep-link] cannot register {scheme}: {e}");
            }
        }
    }

    crate::watch_url_open(on_url_open);
}

pub(crate) fn deliver(app: Option<&AppHandle>, urls: Vec<String>) {
    if urls.is_empty() {
        return;
    }
    *CURRENT.lock().unwrap() = urls.clone();
    if let Some(app) = app {
        app.emit(EVENT, serde_json::json!({ "urls": urls }));
    }
}

extern "C" fn on_url_open(url: *const std::os::raw::c_char) {
    if url.is_null() {
        return;
    }
    let url = unsafe { std::ffi::CStr::from_ptr(url) }.to_string_lossy().into_owned();
    deliver(crate::app::global_handle(), vec![url]);
}

#[cfg(windows)]
fn register(scheme: &str) -> Result<(), String> {
    if crate::register_url_scheme(scheme) { Ok(()) } else { Err(format!("cannot write HKCU\\Software\\Classes\\{scheme}")) }
}

#[cfg(windows)]
fn unregister(scheme: &str) -> Result<(), String> {
    if crate::unregister_url_scheme(scheme) { Ok(()) } else { Err(format!("cannot remove HKCU\\Software\\Classes\\{scheme}")) }
}

#[cfg(windows)]
fn is_registered(scheme: &str) -> bool { crate::url_scheme_registered(scheme) }

#[cfg(target_os = "macos")]
fn register(scheme: &str) -> Result<(), String> { Err(format!("macOS only takes URL schemes from Info.plist: add `{scheme}` to app.deepLink.schemes")) }

#[cfg(target_os = "macos")]
fn unregister(scheme: &str) -> Result<(), String> { Err(format!("macOS only takes URL schemes from Info.plist: remove `{scheme}` from app.deepLink.schemes")) }

#[cfg(target_os = "macos")]
fn is_registered(scheme: &str) -> bool { configured_schemes().iter().any(|configured| configured == scheme) }

#[cfg(all(unix, not(target_os = "macos")))]
use linux::{is_registered, register, unregister};

#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use crate::path::PathResolver;
    use std::path::PathBuf;

    const SECTION: &str = "[Default Applications]";

    fn entry_name(scheme: &str) -> String { format!("{}-{scheme}-handler.desktop", crate::config::identifier()) }

    fn entry_path(scheme: &str) -> Option<PathBuf> { Some(PathResolver::new().data_dir()?.join("applications").join(entry_name(scheme))) }

    fn mimeapps() -> Option<PathBuf> { Some(PathResolver::new().config_dir()?.join("mimeapps.list")) }

    fn key(scheme: &str) -> String { format!("x-scheme-handler/{scheme}") }

    fn launcher() -> Option<PathBuf> { std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok()) }

    fn quoted(path: &str) -> String {
        if !path
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '`' | '$' | '\\' | '\'' | '>' | '<' | '~' | '|' | '&' | ';' | '*' | '?' | '#' | '(' | ')'))
        {
            return path.to_owned();
        }
        let escaped: String = path.chars().flat_map(|c| if matches!(c, '"' | '`' | '$' | '\\') { vec!['\\', c] } else { vec![c] }).collect();
        format!("\"{escaped}\"")
    }

    pub(super) fn register(scheme: &str) -> Result<(), String> {
        let path = entry_path(scheme).ok_or("no data directory")?;
        let launcher = launcher().ok_or("cannot tell which executable is running")?;

        let entry = format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec={} %u\nTerminal=false\nNoDisplay=true\nMimeType={};\n",
            crate::config::product_name(),
            quoted(&launcher.display().to_string()),
            key(scheme)
        );
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, entry).map_err(|e| format!("{}: {e}", path.display()))?;
        set_default(scheme, Some(&entry_name(scheme)))
    }

    pub(super) fn unregister(scheme: &str) -> Result<(), String> {
        if default_for(scheme).as_deref() == Some(entry_name(scheme).as_str()) {
            set_default(scheme, None)?;
        }
        if let Some(path) = entry_path(scheme) {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    pub(super) fn is_registered(scheme: &str) -> bool {
        default_for(scheme).as_deref() == Some(entry_name(scheme).as_str()) && entry_path(scheme).is_some_and(|path| path.is_file())
    }

    fn default_for(scheme: &str) -> Option<String> {
        let text = std::fs::read_to_string(mimeapps()?).ok()?;
        let key = key(scheme);
        let mut in_section = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_section = line == SECTION;
            } else if in_section
                && let Some((name, value)) = line.split_once('=')
                && name.trim() == key
            {
                return value.split(';').map(str::trim).find(|entry| !entry.is_empty()).map(str::to_owned);
            }
        }
        None
    }

    fn set_default(scheme: &str, entry: Option<&str>) -> Result<(), String> {
        let path = mimeapps().ok_or("no config directory")?;
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let key = key(scheme);

        let mut lines: Vec<String> = Vec::new();
        let mut in_section = false;
        let mut placed = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_section = trimmed == SECTION;
                lines.push(line.to_owned());
                if in_section && !placed {
                    if let Some(entry) = entry {
                        lines.push(format!("{key}={entry};"));
                    }
                    placed = true;
                }
                continue;
            }
            if in_section && trimmed.split_once('=').is_some_and(|(name, _)| name.trim() == key) {
                continue;
            }
            lines.push(line.to_owned());
        }
        if !placed && let Some(entry) = entry {
            if lines.last().is_some_and(|last| !last.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(SECTION.to_owned());
            lines.push(format!("{key}={entry};"));
        }

        let mut out = lines.join("\n");
        out.push('\n');
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))
    }
}
