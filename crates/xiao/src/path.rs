use crate::config;
use std::path::PathBuf;

mod location {
    pub const DESKTOP: i32 = 0;
    pub const DOCUMENTS: i32 = 1;
    pub const FONTS: i32 = 2;
    pub const MUSIC: i32 = 4;
    pub const MOVIES: i32 = 5;
    pub const PICTURES: i32 = 6;
    pub const TEMP: i32 = 7;
    pub const HOME: i32 = 8;
    pub const APP_LOCAL_DATA: i32 = 9;
    pub const CACHE: i32 = 10;
    #[cfg(all(unix, not(target_os = "macos")))]
    pub const RUNTIME: i32 = 12;
    pub const DOWNLOAD: i32 = 14;
    pub const GENERIC_CACHE: i32 = 15;
    pub const GENERIC_CONFIG: i32 = 16;
    pub const APP_DATA: i32 = 17;
    pub const APP_CONFIG: i32 = 18;
    pub const PUBLIC_SHARE: i32 = 19;
}

fn qt(location: i32) -> Option<PathBuf> { crate::standard_path(location, None).map(PathBuf::from) }

fn bare(location: i32) -> Option<PathBuf> { crate::standard_path(location, Some("")).map(PathBuf::from) }

fn scoped(location: i32) -> Option<PathBuf> { crate::standard_path(location, Some(config::identifier())).map(PathBuf::from) }

/// Standard directories, from [`AppHandle::path`](crate::AppHandle::path),
/// resolved by `QStandardPaths`.
///
/// The `app_*` directories end in the app's `identifier`, for example
/// `~/.config/<identifier>` on Linux or `AppData\Roaming\<identifier>` on
/// Windows. None are created until [`ensure`](Self::ensure) is called.
pub struct PathResolver(());

impl PathResolver {
    pub(crate) fn new() -> Self { Self(()) }

    pub fn home_dir(&self) -> Option<PathBuf> { qt(location::HOME) }

    pub fn config_dir(&self) -> Option<PathBuf> { qt(location::GENERIC_CONFIG) }

    /// The roaming data directory on Windows; the same as [`local_data_dir`](Self::local_data_dir) elsewhere.
    pub fn data_dir(&self) -> Option<PathBuf> { bare(location::APP_DATA) }

    pub fn local_data_dir(&self) -> Option<PathBuf> { bare(location::APP_LOCAL_DATA) }

    pub fn cache_dir(&self) -> Option<PathBuf> { qt(location::GENERIC_CACHE) }

    /// `XDG_RUNTIME_DIR` on Linux and other non-macOS unix systems; `None` elsewhere.
    pub fn runtime_dir(&self) -> Option<PathBuf> {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            qt(location::RUNTIME)
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        {
            None
        }
    }

    pub fn temp_dir(&self) -> Option<PathBuf> { qt(location::TEMP) }

    pub fn desktop_dir(&self) -> Option<PathBuf> { qt(location::DESKTOP) }

    pub fn document_dir(&self) -> Option<PathBuf> { qt(location::DOCUMENTS) }

    pub fn download_dir(&self) -> Option<PathBuf> { qt(location::DOWNLOAD) }

    pub fn picture_dir(&self) -> Option<PathBuf> { qt(location::PICTURES) }

    pub fn public_dir(&self) -> Option<PathBuf> { qt(location::PUBLIC_SHARE) }

    pub fn audio_dir(&self) -> Option<PathBuf> { qt(location::MUSIC) }

    pub fn video_dir(&self) -> Option<PathBuf> { qt(location::MOVIES) }

    pub fn font_dir(&self) -> Option<PathBuf> { qt(location::FONTS) }

    /// The directory containing the running executable.
    pub fn executable_dir(&self) -> Option<PathBuf> {
        crate::application_dir().map(PathBuf::from).or_else(|| std::env::current_exe().ok()?.parent().map(PathBuf::from))
    }

    pub fn app_config_dir(&self) -> Option<PathBuf> { scoped(location::APP_CONFIG) }

    pub fn app_data_dir(&self) -> Option<PathBuf> { scoped(location::APP_DATA) }

    pub fn app_local_data_dir(&self) -> Option<PathBuf> { scoped(location::APP_LOCAL_DATA) }

    pub fn app_cache_dir(&self) -> Option<PathBuf> { scoped(location::CACHE) }

    /// `~/Library/Logs/<identifier>` on macOS, `logs` in [`app_local_data_dir`](Self::app_local_data_dir) elsewhere.
    pub fn app_log_dir(&self) -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            Some(self.home_dir()?.join("Library/Logs").join(config::identifier()))
        }
        #[cfg(not(target_os = "macos"))]
        {
            Some(self.app_local_data_dir()?.join("logs"))
        }
    }

    /// Where `bundle.resources` is installed, each entry under its own relative path: the executable's directory on
    /// Windows and in cargo's `target/` (dev builds), `Contents/Resources` on macOS, `/usr/lib/<identifier>` on Linux
    /// (inside the AppImage when running from one).
    pub fn resource_dir(&self) -> Option<PathBuf> { config::resource_dir() }

    /// A file in [`resource_dir`](Self::resource_dir), e.g. `resources/hello.txt` for a `"resources"` entry. `None`
    /// unless the path lies inside one of the `bundle.resources` entries, so files the bundle ships alongside them
    /// (Qt's libraries, plugins, QML) never resolve.
    pub fn resolve_resource(&self, relative: &str) -> Option<PathBuf> { config::resolve_resource(relative) }

    /// Creates `dir` and its parents if needed, and returns it.
    pub fn ensure(&self, dir: Option<PathBuf>) -> std::io::Result<PathBuf> {
        let dir = dir.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "directory is not defined on this platform"))?;
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// A directory by the name the page uses: `"home"`, `"appData"`, `"download"` and so on.
    pub fn by_name(&self, name: &str) -> Option<PathBuf> {
        match name {
            "home" => self.home_dir(),
            "config" => self.config_dir(),
            "data" => self.data_dir(),
            "localData" => self.local_data_dir(),
            "cache" => self.cache_dir(),
            "runtime" => self.runtime_dir(),
            "temp" => self.temp_dir(),
            "desktop" => self.desktop_dir(),
            "document" => self.document_dir(),
            "download" => self.download_dir(),
            "picture" => self.picture_dir(),
            "public" => self.public_dir(),
            "audio" => self.audio_dir(),
            "video" => self.video_dir(),
            "font" => self.font_dir(),
            "executable" => self.executable_dir(),
            "appConfig" => self.app_config_dir(),
            "appData" => self.app_data_dir(),
            "appLocalData" => self.app_local_data_dir(),
            "appCache" => self.app_cache_dir(),
            "appLog" => self.app_log_dir(),
            "resource" => self.resource_dir(),
            _ => None,
        }
    }

    /// Every directory [`by_name`](Self::by_name) knows, with its name.
    pub fn all(&self) -> Vec<(&'static str, Option<PathBuf>)> {
        const NAMES: &[&str] = &[
            "home",
            "config",
            "data",
            "localData",
            "cache",
            "runtime",
            "temp",
            "desktop",
            "document",
            "download",
            "picture",
            "public",
            "audio",
            "video",
            "font",
            "executable",
            "appConfig",
            "appData",
            "appLocalData",
            "appCache",
            "appLog",
            "resource",
        ];
        NAMES.iter().map(|n| (*n, self.by_name(n))).collect()
    }
}
