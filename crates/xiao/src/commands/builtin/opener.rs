use crate::app::AppHandle;
use crate::commands::XiaoCommand;
use crate::opener::OpenMode;
use serde_json::Value;
use std::path::{Path, PathBuf};

const RUNNABLE: &[&str] = &[
    "exe", "com", "bat", "cmd", "msi", "msp", "scr", "pif", "cpl", "hta", "lnk", "url", "reg", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "jar",
    "desktop", "sh", "run", "appimage", "command", "terminal", "tool", "app", "pkg",
];

pub struct Opener;

impl XiaoCommand for Opener {
    type Args = Value;
    type Output = Value;

    fn name() -> &'static str { "xiao:opener" }

    fn execute(app: &AppHandle, args: Self::Args) -> Result<Self::Output, String> {
        let opener = app.opener();

        if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
            let scheme = url.trim().split_once(':').map(|(scheme, _)| scheme.to_ascii_lowercase()).unwrap_or_default();
            if !scheme_allowed(app, &scheme) {
                return Err(format!(
                    "the page may not open `{scheme}:` links: only http, https, mailto, the app's own deep-link schemes, and those in app.security.opener.schemes"
                ));
            }
            opener.open_url(url)?;
            return Ok(Value::Null);
        }

        let path = args.get("path").and_then(|v| v.as_str()).filter(|path| !path.is_empty()).ok_or("missing `url` or `path`")?;
        let mode = match args.get("mode").and_then(|v| v.as_str()) {
            Some(name) => OpenMode::from_name(name).ok_or_else(|| format!("unknown open mode `{name}`"))?,
            None => OpenMode::default(),
        };

        let resolved = in_scope(app, Path::new(path))?;
        if mode == OpenMode::Open && runs(&resolved) {
            return Err(format!("the page may not open {}: it would run rather than open, reveal it instead", resolved.display()));
        }

        opener.open_path(&resolved, mode)?;
        Ok(Value::Null)
    }
}

fn scheme_allowed(app: &AppHandle, scheme: &str) -> bool {
    if matches!(scheme, "http" | "https" | "mailto") || crate::config::opener_schemes().iter().any(|allowed| allowed == scheme) {
        return true;
    }
    crate::deep_link::configured_schemes().iter().any(|own| own == scheme) || app.deep_link().is_registered(scheme)
}

fn in_scope(app: &AppHandle, path: &Path) -> Result<PathBuf, String> {
    let names = crate::config::opener_paths();
    if names.is_empty() {
        return Err("the page may not open paths: list the directories it may open in under app.security.opener.paths".to_owned());
    }

    let resolved = std::path::absolute(path).and_then(|path| path.canonicalize()).map_err(|e| format!("{}: {e}", path.display()))?;
    let resolver = app.path();
    let inside = names
        .iter()
        .flat_map(|name| match name.as_str() {
            "resource" => crate::config::resource_scopes(),
            _ => resolver.by_name(name).and_then(|dir| dir.canonicalize().ok()).into_iter().collect(),
        })
        .any(|dir| resolved.starts_with(&dir));
    if inside { Ok(resolved) } else { Err(format!("{} is outside the directories in app.security.opener.paths ({})", resolved.display(), names.join(", "))) }
}

fn runs(path: &Path) -> bool {
    if path.is_dir() {
        return path.extension().is_some_and(|ext| matches!(ext.to_ascii_lowercase().to_str(), Some("app")));
    }
    let by_name = path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| RUNNABLE.contains(&ext.to_ascii_lowercase().as_str()));
    by_name || executable_bit(path)
}

#[cfg(unix)]
fn executable_bit(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable_bit(_path: &Path) -> bool { false }
