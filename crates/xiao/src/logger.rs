use log::{LevelFilter, Log, Metadata, Record};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub(crate) const EVENT: &str = "log://log";

type Filter = Box<dyn Fn(&Metadata) -> bool + Send + Sync>;

/// Where a [`Logger`] writes.
pub enum Target {
    Stdout,
    Stderr,
    /// `<file_name>.log` in [`PathResolver::app_log_dir`](crate::PathResolver::app_log_dir).
    /// `file_name` defaults to the product name.
    LogDir {
        file_name: Option<String>,
    },
    /// `<file_name>.log` in `path`.
    Folder {
        path: PathBuf,
        file_name: Option<String>,
    },
    /// The page, as `log://log` events. `log.attachConsole()` in `@xiao/api` prints them.
    Webview,
}

/// What happens to a log file that reaches the maximum size.
pub enum RotationStrategy {
    /// Rename it to `<name>_<timestamp>.log` and keep every old file.
    KeepAll,
    /// Delete it and start over.
    KeepOne,
    /// Rename it and keep the newest `n` old files.
    KeepSome(usize),
}

/// Which clock timestamps log lines.
pub enum TimezoneStrategy {
    UseUtc,
    UseLocal,
}

/// A logger for the `log` crate, installed with [`Shell::logger`](crate::Shell::logger).
///
/// ```ignore
/// xiao::Logger::new()
///     .level(log::LevelFilter::Info)
///     .clear_targets()
///     .target(xiao::LogTarget::LogDir { file_name: Some("myapp".into()) })
///     .max_file_size(8_000_000)
///     .rotation_strategy(xiao::RotationStrategy::KeepSome(5))
/// ```
pub struct Logger {
    level: LevelFilter,
    levels: HashMap<String, LevelFilter>,
    filters: Vec<Filter>,
    targets: Vec<Target>,
    max_file_size: u64,
    rotation: RotationStrategy,
    timezone: TimezoneStrategy,
}

impl Default for Logger {
    fn default() -> Self {
        Self {
            level: LevelFilter::Trace,
            levels: HashMap::new(),
            filters: Vec::new(),
            targets: vec![Target::Stdout, Target::LogDir { file_name: None }],
            max_file_size: 40_000,
            rotation: RotationStrategy::KeepOne,
            timezone: TimezoneStrategy::UseUtc,
        }
    }
}

impl Logger {
    /// Logs every level to stdout and the log directory, in UTC, rotating
    /// files at 40 KB and keeping one.
    pub fn new() -> Self { Self::default() }

    pub fn level(mut self, level: LevelFilter) -> Self {
        self.level = level;
        self
    }

    /// Sets the level for `target` and the modules under it, such as `"hyper"` for `hyper::client`.
    pub fn level_for(mut self, target: impl Into<String>, level: LevelFilter) -> Self {
        self.levels.insert(target.into(), level);
        self
    }

    /// Drops records for which `filter` returns `false`.
    pub fn filter<F>(mut self, filter: F) -> Self
    where F: Fn(&Metadata) -> bool + Send + Sync + 'static {
        self.filters.push(Box::new(filter));
        self
    }

    pub fn target(mut self, target: Target) -> Self {
        self.targets.push(target);
        self
    }

    pub fn targets(mut self, targets: impl IntoIterator<Item = Target>) -> Self {
        self.targets.extend(targets);
        self
    }

    /// Removes the default targets and any added so far.
    pub fn clear_targets(mut self) -> Self {
        self.targets.clear();
        self
    }

    /// The size in bytes at which a log file is rotated.
    pub fn max_file_size(mut self, bytes: u64) -> Self {
        self.max_file_size = bytes;
        self
    }

    pub fn rotation_strategy(mut self, rotation: RotationStrategy) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn timezone_strategy(mut self, timezone: TimezoneStrategy) -> Self {
        self.timezone = timezone;
        self
    }

    pub(crate) fn install(self) -> Result<(), log::SetLoggerError> {
        let mut max = self.level;
        for level in self.levels.values() {
            max = max.max(*level);
        }

        let mut sinks = Vec::new();
        for target in self.targets {
            let sink = match target {
                Target::Stdout => Sink::Stdout,
                Target::Stderr => Sink::Stderr,
                Target::Webview => Sink::Webview,
                Target::LogDir { file_name } => match crate::path::PathResolver::new().app_log_dir() {
                    Some(dir) => Sink::File(Mutex::new(LogFile::new(dir, file_name))),
                    None => {
                        eprintln!("[log] no log directory on this platform, the LogDir target is skipped");
                        continue;
                    }
                },
                Target::Folder { path, file_name } => Sink::File(Mutex::new(LogFile::new(path, file_name))),
            };
            sinks.push(sink);
        }

        let logger = AppLogger {
            level: self.level,
            levels: self.levels,
            filters: self.filters,
            sinks,
            max_file_size: self.max_file_size,
            rotation: self.rotation,
            local: matches!(self.timezone, TimezoneStrategy::UseLocal),
        };
        log::set_boxed_logger(Box::new(logger))?;
        log::set_max_level(max);
        Ok(())
    }
}

enum Sink {
    Stdout,
    Stderr,
    Webview,
    File(Mutex<LogFile>),
}

struct LogFile {
    dir: PathBuf,
    name: String,
    file: Option<File>,
    size: u64,
}

impl LogFile {
    fn new(dir: PathBuf, name: Option<String>) -> Self {
        Self { dir, name: name.unwrap_or_else(|| crate::config::product_name().to_owned()), file: None, size: 0 }
    }

    fn path(&self) -> PathBuf { self.dir.join(format!("{}.log", self.name)) }

    fn write(&mut self, line: &str, max: u64, rotation: &RotationStrategy, local: bool) {
        if self.file.is_none() {
            let _ = std::fs::create_dir_all(&self.dir);
            match OpenOptions::new().create(true).append(true).open(self.path()) {
                Ok(file) => {
                    self.size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
                    self.file = Some(file);
                }
                Err(e) => {
                    eprintln!("[log] cannot open {}: {e}", self.path().display());
                    return;
                }
            }
        }

        if max > 0 && self.size + line.len() as u64 > max && self.size > 0 {
            self.rotate(rotation, local);
            return self.write(line, max, rotation, local);
        }

        if let Some(file) = self.file.as_mut()
            && file.write_all(line.as_bytes()).is_ok()
        {
            self.size += line.len() as u64;
        }
    }

    fn rotate(&mut self, rotation: &RotationStrategy, local: bool) {
        self.file = None;
        let current = self.path();
        match rotation {
            RotationStrategy::KeepOne => {
                let _ = std::fs::remove_file(&current);
            }
            RotationStrategy::KeepAll | RotationStrategy::KeepSome(_) => {
                let stamp = crate::timestamp(local).replace([' ', ':', '.'], "-");
                let _ = std::fs::rename(&current, self.dir.join(format!("{}_{stamp}.log", self.name)));
            }
        }

        if let RotationStrategy::KeepSome(keep) = rotation {
            let prefix = format!("{}_", self.name);
            let mut rotated: Vec<PathBuf> = std::fs::read_dir(&self.dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    name.starts_with(&prefix) && name.ends_with(".log")
                })
                .collect();
            rotated.sort();
            while rotated.len() > *keep {
                let _ = std::fs::remove_file(rotated.remove(0));
            }
        }
    }
}

struct AppLogger {
    level: LevelFilter,
    levels: HashMap<String, LevelFilter>,
    filters: Vec<Filter>,
    sinks: Vec<Sink>,
    max_file_size: u64,
    rotation: RotationStrategy,
    local: bool,
}

impl AppLogger {
    fn level_of(&self, target: &str) -> LevelFilter {
        self.levels
            .iter()
            .filter(|(prefix, _)| target == prefix.as_str() || target.starts_with(&format!("{prefix}::")))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, level)| *level)
            .unwrap_or(self.level)
    }
}

impl Log for AppLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level_of(metadata.target()) && self.filters.iter().all(|filter| filter(metadata))
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let line = format!("[{}][{}][{}] {}\n", crate::timestamp(self.local), record.target(), record.level(), record.args());
        for sink in &self.sinks {
            match sink {
                Sink::Stdout => {
                    let _ = std::io::stdout().lock().write_all(line.as_bytes());
                }
                Sink::Stderr => {
                    let _ = std::io::stderr().lock().write_all(line.as_bytes());
                }
                Sink::Webview => {
                    if let Some(app) = crate::app::global_handle() {
                        let payload = serde_json::json!({ "level": record.level().as_str(), "target": record.target(), "message": record.args().to_string() });
                        app.emit(EVENT, payload);
                    }
                }
                Sink::File(file) => {
                    if let Ok(mut file) = file.lock() {
                        file.write(&line, self.max_file_size, &self.rotation, self.local);
                    }
                }
            }
        }
    }

    fn flush(&self) {
        let _ = std::io::stdout().flush();
        for sink in &self.sinks {
            if let Sink::File(file) = sink
                && let Some(file) = file.lock().ok().as_mut().and_then(|file| file.file.as_mut())
            {
                let _ = file.flush();
            }
        }
    }
}
