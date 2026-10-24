use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

static RAW: OnceLock<&'static str> = OnceLock::new();
static PARSED: OnceLock<Value> = OnceLock::new();

pub(crate) fn install(config_json: &'static str) { let _ = RAW.set(config_json); }

pub(crate) fn raw() -> &'static str { RAW.get().copied().unwrap_or("{}") }

pub(crate) fn config() -> &'static Value { PARSED.get_or_init(|| serde_json::from_str(raw()).unwrap_or(Value::Null)) }

pub(crate) fn product_name() -> &'static str { config()["productName"].as_str().unwrap_or("app") }

pub(crate) fn version() -> &'static str { config()["version"].as_str().unwrap_or("0.0.0") }

pub(crate) fn identifier() -> &'static str { config()["identifier"].as_str().unwrap_or("com.example.app") }

pub(crate) fn single_instance_enabled() -> bool { config()["app"]["singleInstance"].as_bool().unwrap_or(true) }

pub(crate) fn match_gpu() -> bool { config()["app"]["webEngine"]["matchGpu"].as_bool().unwrap_or(false) }

pub(crate) fn page_may(capability: &str) -> bool { config()["app"]["security"]["page"][capability].as_bool().unwrap_or(false) }

pub(crate) fn page_denied(capability: &str) -> String {
    format!("the page may not do this: set app.security.page.{capability} to true in xiao.conf.json to allow it")
}

pub(crate) fn opener_schemes() -> Vec<String> {
    config()["app"]["security"]["opener"]["schemes"]
        .as_array()
        .map(|schemes| schemes.iter().filter_map(|scheme| scheme.as_str()).map(str::to_ascii_lowercase).collect())
        .unwrap_or_default()
}

pub(crate) fn opener_paths() -> Vec<String> {
    config()["app"]["security"]["opener"]["paths"]
        .as_array()
        .map(|names| names.iter().filter_map(|name| name.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

pub(crate) fn resource_dir() -> Option<PathBuf> {
    static RESOLVED: OnceLock<Option<PathBuf>> = OnceLock::new();
    RESOLVED.get_or_init(|| locate_resource_dir().map(|path| canonical(&path).unwrap_or(path))).clone()
}

fn locate_resource_dir() -> Option<PathBuf> {
    let exe_dir = crate::path::PathResolver::new().executable_dir()?;
    if cfg!(windows) || exe_dir.join(".cargo-lock").exists() {
        return Some(exe_dir);
    }
    if cfg!(target_os = "macos") {
        return Some(exe_dir.join("../Resources"));
    }

    let installed = exe_dir.join("../lib").join(identifier());
    if installed.is_dir() {
        return Some(installed);
    }
    match std::env::var_os("APPDIR") {
        Some(appdir) => Some(PathBuf::from(appdir).join("usr/lib").join(identifier())),
        None => Some(PathBuf::from("/usr/lib").join(identifier())),
    }
}

fn declared_resources() -> Vec<PathBuf> {
    config()["bundle"]["resources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.as_str())
        .map(|entry| resource_relpath(Path::new(entry)))
        .filter(|relative| !relative.as_os_str().is_empty())
        .collect()
}

fn resource_relpath(entry: &Path) -> PathBuf {
    entry
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_os_string()),
            Component::ParentDir => Some("_up_".into()),
            Component::RootDir => Some("_root_".into()),
            Component::CurDir | Component::Prefix(_) => None,
        })
        .collect()
}

fn canonical(path: &std::path::Path) -> Option<PathBuf> {
    let resolved = path.canonicalize().ok()?;
    #[cfg(windows)]
    {
        let text = resolved.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            if rest.as_bytes().get(1) == Some(&b':') {
                return Some(PathBuf::from(rest));
            }
        }
    }
    Some(resolved)
}

pub(crate) fn is_relative_below(relative: &str) -> bool {
    !relative.is_empty()
        && std::path::Path::new(relative).components().all(|part| matches!(part, std::path::Component::Normal(_) | std::path::Component::CurDir))
}

const EMBEDDED: &str = ":/xiao";

pub(crate) const EMBEDDED_FRONTEND: &str = ":/xiao/dist";

pub(crate) fn frontend_url() -> Option<String> { crate::resource_exists(&format!("{EMBEDDED_FRONTEND}/index.html")).then(|| "xiao://localhost/".to_owned()) }

pub(crate) fn resolve_resource(relative: &str) -> Option<PathBuf> {
    if !is_relative_below(relative) {
        return None;
    }

    let joined = resource_dir()?.join(relative);
    let real = joined.canonicalize().ok()?;
    resource_scopes().iter().any(|scope| real.starts_with(scope)).then(|| canonical(&joined)).flatten()
}

pub(crate) fn resource_scopes() -> Vec<PathBuf> {
    let Some(root) = resource_dir() else { return Vec::new() };
    declared_resources().into_iter().filter_map(|entry| root.join(entry).canonicalize().ok()).collect()
}

fn bundle_icons() -> Vec<&'static str> {
    match &config()["bundle"]["icon"] {
        Value::Array(items) => items.iter().filter_map(|item| item.as_str()).collect(),
        Value::String(one) => vec![one.as_str()],
        _ => Vec::new(),
    }
}

fn largest_png() -> Option<&'static str> {
    fn pixels(path: &str) -> u32 {
        let stem = path.rsplit(['/', '\\']).next().unwrap_or(path);
        stem.strip_suffix(".png")
            .and_then(|name| name.split_once('x'))
            .filter(|(width, height)| width == height)
            .and_then(|(width, _)| width.parse().ok())
            .unwrap_or(u32::MAX)
    }

    bundle_icons().into_iter().filter(|path| path.ends_with(".png")).max_by_key(|path| pixels(path))
}

pub(crate) fn tray_icon_url() -> String { embedded_icon_name().map(|icon| format!("qrc{EMBEDDED}/icons/{icon}")).unwrap_or_default() }

pub(crate) fn window_icon_path() -> Option<String> { embedded_icon_name().map(|icon| format!("{EMBEDDED}/icons/{icon}")) }

fn embedded_icon_name() -> Option<&'static str> {
    config()["app"]["trayIcon"]["iconPath"]
        .as_str()
        .filter(|path| !path.is_empty())
        .or_else(largest_png)
        .map(|path| path.rsplit(['/', '\\']).next().unwrap_or(path))
        .filter(|icon| crate::resource_exists(&format!("{EMBEDDED}/icons/{icon}")))
}

fn render_csp(value: &Value) -> Option<String> {
    match value {
        Value::String(policy) if !policy.trim().is_empty() => Some(policy.trim().to_owned()),
        Value::Object(directives) => {
            let rendered: Vec<String> = directives
                .iter()
                .filter_map(|(directive, sources)| {
                    let sources = match sources {
                        Value::String(one) => one.trim().to_owned(),
                        Value::Array(list) => list.iter().filter_map(|source| source.as_str()).collect::<Vec<_>>().join(" "),
                        _ => return None,
                    };
                    Some(if sources.is_empty() { directive.clone() } else { format!("{directive} {sources}") })
                })
                .collect();
            (!rendered.is_empty()).then(|| rendered.join("; "))
        }
        _ => None,
    }
}

pub(crate) fn active_csp() -> Option<String> {
    let security = &config()["app"]["security"];
    let key = if crate::bridge::is_dev_mode() { "devCsp" } else { "csp" };
    render_csp(&security[key])
}

pub(crate) fn web_storage() -> Value {
    let paths = crate::path::PathResolver::new();
    let data = paths.ensure(paths.app_local_data_dir()).ok();
    let cache = paths.ensure(paths.app_cache_dir()).ok().or_else(|| data.clone());
    let downloads = paths.download_dir().or_else(|| data.clone());

    let path = |dir: &Option<PathBuf>| dir.as_ref().map(|dir| dir.display().to_string()).unwrap_or_default();

    json!({
        "storageName": product_name(),
        "storagePath": path(&data),
        "cachePath": path(&cache),
        "downloadPath": path(&downloads),
    })
}
