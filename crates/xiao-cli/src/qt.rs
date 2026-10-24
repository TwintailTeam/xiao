use crate::util::*;
use std::path::{Path, PathBuf};

pub const QML_MODULES: &[&str] = &["QtQuick", "QtQuick.2", "QtQml", "QtWebEngine", "QtWebChannel", "Qt/labs/platform", "QtCore"];

pub fn qmake() -> Result<String, String> {
    match std::env::var_os("XIAO_TARGET_QT") {
        Some(prefix) => target_qmake(Path::new(&prefix)).map(|path| path.display().to_string()),
        None => Ok(host_qmake()),
    }
}

pub fn cargo_env() -> Result<Vec<(&'static str, String)>, String> {
    let mut env = vec![("QMAKE", qmake()?)];
    if let Some(prefix) = std::env::var_os("XIAO_TARGET_QT")
        && disables_zstd(Path::new(&prefix))
    {
        let options = match std::env::var("CXX_QT_AUTORCC_OPTIONS") {
            Ok(existing) if !existing.is_empty() => format!("{existing}:--no-zstd"),
            _ => "--no-zstd".to_owned(),
        };
        env.push(("CXX_QT_AUTORCC_OPTIONS", options));
    }
    Ok(env)
}

fn disables_zstd(prefix: &Path) -> bool {
    std::fs::read_to_string(prefix.join("mkspecs/qconfig.pri")).is_ok_and(|text| {
        text.lines().filter_map(|line| line.strip_prefix("QT.global.disabled_features")).any(|rest| rest.split_whitespace().any(|feature| feature == "zstd"))
    })
}

fn host_qmake() -> String {
    if let Ok(explicit) = std::env::var("QMAKE") {
        return explicit;
    }
    ["qmake6", "qmake"].into_iter().find(|name| which(name)).unwrap_or("qmake6").to_owned()
}

fn target_qmake(prefix: &Path) -> Result<PathBuf, String> {
    if cfg!(windows) {
        return Err("XIAO_TARGET_QT is not used on Windows, unset it".to_owned());
    }
    if !prefix.join("include/QtCore").is_dir() || !prefix.join("lib").is_dir() {
        return Err(format!("XIAO_TARGET_QT={} is not a Qt installation (no include/QtCore and lib/)", prefix.display()));
    }

    let qconfig = prefix.join("mkspecs/qconfig.pri");
    let target_version = std::fs::read_to_string(&qconfig)
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| line.strip_prefix("QT_VERSION").and_then(|rest| rest.trim_start().strip_prefix('=')).map(|v| v.trim().to_owned()))
        })
        .ok_or_else(|| format!("cannot read QT_VERSION from {}", qconfig.display()))?;

    let host = host_qmake();
    let query = |key: &str| -> Result<String, String> {
        let output = std::process::Command::new(&host).args(["-query", key]).output().map_err(|e| format!("{host} -query {key}: {e}"))?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let host_version = query("QT_VERSION")?;
    let minor = |version: &str| version.split('.').take(2).collect::<Vec<_>>().join(".");
    if minor(&host_version) != minor(&target_version) {
        return Err(format!("host Qt is {host_version}, XIAO_TARGET_QT is {target_version}: the minor versions must match"));
    }

    let libexecs = if prefix.join("libexec").is_dir() { "libexec" } else { "bin" };
    let prefix = prefix.display().to_string();
    let entries = [
        ("QT_VERSION", target_version),
        ("QT_INSTALL_PREFIX", prefix.clone()),
        ("QT_INSTALL_ARCHDATA", prefix.clone()),
        ("QT_INSTALL_DATA", prefix.clone()),
        ("QT_INSTALL_HEADERS", format!("{prefix}/include")),
        ("QT_INSTALL_LIBS", format!("{prefix}/lib")),
        ("QT_INSTALL_LIBEXECS", format!("{prefix}/{libexecs}")),
        ("QT_INSTALL_BINS", format!("{prefix}/bin")),
        ("QT_INSTALL_PLUGINS", format!("{prefix}/plugins")),
        ("QT_INSTALL_QML", format!("{prefix}/qml")),
        ("QT_INSTALL_TRANSLATIONS", format!("{prefix}/translations")),
        ("QT_HOST_BINS", query("QT_HOST_BINS")?),
        ("QT_HOST_LIBEXECS", query("QT_HOST_LIBEXECS")?),
    ];

    let mut script = String::from("#!/bin/sh\n[ \"$1\" = \"-query\" ] || exit 1\ncase \"${2%/get}\" in\n");
    for (key, value) in &entries {
        script.push_str(&format!("    {key}) echo '{value}' ;;\n"));
    }
    script.push_str("    \"\")\n");
    for (key, value) in &entries {
        script.push_str(&format!("        echo '{key}:{value}'\n"));
    }
    script.push_str("        ;;\n    *) exit 1 ;;\nesac\n");

    let path = std::env::temp_dir().join(format!("xiao-qmake-{:016x}", short_hash(&prefix)));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(script.as_str()) {
        write(&path, &script)?;
        make_executable(&path)?;
    }
    Ok(path)
}

fn short_hash(text: &str) -> u64 { text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)) }

pub struct QtPaths {
    pub headers: PathBuf,
    pub libs: PathBuf,
    pub bins: PathBuf,
    pub plugins: PathBuf,
    pub qml: PathBuf,
    pub libexecs: PathBuf,
    pub data: PathBuf,
    pub translations: PathBuf,
}

impl QtPaths {
    pub fn query() -> Result<Self, String> {
        let qmake = qmake()?;
        let output =
            std::process::Command::new(&qmake).arg("-query").output().map_err(|e| format!("{qmake} -query: {e} (set QMAKE to the Qt you build with)"))?;
        if !output.status.success() {
            return Err(format!("{qmake} -query failed"));
        }

        let text = String::from_utf8_lossy(&output.stdout);
        let find = |key: &str| -> Result<PathBuf, String> {
            text.lines()
                .find_map(|line| line.strip_prefix(key).and_then(|rest| rest.strip_prefix(':')))
                .map(|value| PathBuf::from(value.trim()))
                .ok_or_else(|| format!("{qmake} -query did not report {key}"))
        };

        Ok(Self {
            headers: find("QT_INSTALL_HEADERS")?,
            libs: find("QT_INSTALL_LIBS")?,
            bins: find("QT_INSTALL_BINS")?,
            plugins: find("QT_INSTALL_PLUGINS")?,
            qml: find("QT_INSTALL_QML")?,
            libexecs: find("QT_INSTALL_LIBEXECS")?,
            data: find("QT_INSTALL_DATA")?,
            translations: find("QT_INSTALL_TRANSLATIONS")?,
        })
    }

    pub fn proprietary_codecs(&self) -> Option<bool> {
        let roots = [self.headers.join("QtWebEngineCore"), self.libs.join("QtWebEngineCore.framework/Headers")];
        let versions = roots.iter().filter_map(|root| std::fs::read_dir(root).ok()).flat_map(|entries| entries.flatten());
        let header = versions.map(|version| version.path().join("QtWebEngineCore/private/qtwebenginecore-config_p.h")).find(|header| header.is_file())?;
        let text = std::fs::read_to_string(header).ok()?;
        text.lines().find_map(|line| line.strip_prefix("#define QT_FEATURE_webengine_proprietary_codecs ")).map(|value| value.trim() == "1")
    }
}

const QML_UNUSED: &[&str] = &["VirtualKeyboard", "Pdf", "Particles", "Timeline", "VectorImage", "LocalStorage", "tooling"];

const IMAGE_FORMATS: &[&str] = &["qgif", "qico", "qjpeg", "qsvg", "qwebp", "qicns"];

pub struct Prune {
    pub locales: Vec<String>,
}

impl Prune {
    pub fn from_conf(conf: &crate::conf::Conf) -> Self {
        let mut locales = conf.strings_at(&["bundle", "webengineLocales"]);
        if !locales.iter().any(|locale| locale == "en-US") {
            locales.push("en-US".to_owned());
        }
        Self { locales }
    }

    fn keeps(&self, path: &Path, release_only: bool, siblings: &[String]) -> bool {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let parent = path.parent().and_then(|parent| parent.file_name()).unwrap_or_default().to_string_lossy();

        if path.is_dir() {
            return !QML_UNUSED.contains(&name.as_ref()) && (!release_only || !name.ends_with(".dSYM"));
        }
        if release_only && is_debug_artifact(&name, siblings) {
            return false;
        }
        match parent.as_ref() {
            "imageformats" => {
                let stem = name.split('.').next().unwrap_or_default();
                IMAGE_FORMATS.contains(&stem.strip_prefix("lib").unwrap_or(stem))
            }
            "platforminputcontexts" => !name.contains("virtualkeyboard"),
            _ => !self.drops_webengine_file(path),
        }
    }

    pub fn drops_webengine_file(&self, path: &Path) -> bool {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let in_locales = path.parent().and_then(|parent| parent.file_name()).is_some_and(|parent| parent == "qtwebengine_locales");
        name.contains("devtools_resources") || (in_locales && !self.locales.iter().any(|locale| name.as_ref() == format!("{locale}.pak")))
    }
}

pub fn copy_qt(from: &Path, to: &Path, release_only: bool, prune: &Prune) -> Result<(), String> {
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    let names: Vec<String> = entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();

    for name in &names {
        let src = from.join(name);
        if !prune.keeps(&src, release_only, &names) {
            continue;
        }
        if src.is_dir() {
            copy_qt(&src, &to.join(name), release_only, prune)?;
        } else {
            copy_file(&src, &to.join(name))?;
        }
    }
    Ok(())
}

fn is_debug_artifact(name: &str, siblings: &[String]) -> bool {
    if name.ends_with(".pdb") || name.ends_with("_debug") || name.contains("_debug.") || name.contains(".debug.") {
        return true;
    }
    name.strip_suffix("d.dll").is_some_and(|release| siblings.iter().any(|sibling| sibling.eq_ignore_ascii_case(&format!("{release}.dll"))))
}
