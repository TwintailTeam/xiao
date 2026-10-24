use std::path::{Path, PathBuf};

const BLOCKED_SCHEMES: &[&str] = &["javascript", "data", "vbscript", "about"];

/// How [`Opener::open_path`] handles a path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpenMode {
    /// Opens it with the application the desktop associates with it.
    #[default]
    Open,
    /// Shows it selected in the file manager, or opens its folder if no file manager can select it.
    Reveal,
}

impl OpenMode {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "open" => Some(Self::Open),
            "reveal" => Some(Self::Reveal),
            _ => None,
        }
    }
}

/// Opens URLs, files and folders with the desktop's applications, from
/// [`AppHandle::opener`](crate::AppHandle::opener).
///
/// `Ok` means the desktop accepted the request, not that anything was shown.
pub struct Opener(());

impl Opener {
    pub(crate) fn new() -> Self { Self(()) }

    /// Opens `url` with the application registered for its scheme.
    /// `javascript:`, `data:`, `vbscript:` and `about:` are refused.
    pub fn open_url(&self, url: &str) -> Result<(), String> {
        let url = check_url(url)?;
        crate::app::trace!("[opener] url {url}");
        match crate::open_url(url) {
            true => Ok(()),
            false => Err(format!("nothing is registered to open `{url}`")),
        }
    }

    /// Opens or reveals an existing file or folder.
    pub fn open_path(&self, path: impl AsRef<Path>, mode: OpenMode) -> Result<(), String> {
        let path = existing(path.as_ref())?;
        crate::app::trace!("[opener] {mode:?} {}", path.display());

        if mode == OpenMode::Reveal && !path.is_dir() {
            if crate::reveal_path(&path) {
                return Ok(());
            }
            crate::app::trace!("[opener] no file manager answered, opening the folder instead");
        }

        let target = match mode {
            OpenMode::Reveal if !path.is_dir() => path.parent().unwrap_or(&path),
            _ => path.as_path(),
        };

        match crate::open_path(target) {
            true => Ok(()),
            false => Err(format!("the desktop refused to open {}", target.display())),
        }
    }
}

fn check_url(url: &str) -> Result<&str, String> {
    let url = url.trim();
    let scheme = url.split_once(':').map(|(scheme, _)| scheme).unwrap_or_default();

    if scheme.len() < 2
        || !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return Err(format!("`{url}` is not a URL: it has no scheme"));
    }
    if BLOCKED_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()) {
        return Err(format!("the `{scheme}` scheme cannot be opened"));
    }
    Ok(url)
}

fn existing(path: &Path) -> Result<PathBuf, String> {
    let path = std::path::absolute(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    Ok(path)
}
