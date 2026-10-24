use crate::conf::Conf;
use crate::qt::{Prune, QML_MODULES, QtPaths, copy_qt};
use crate::util::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

const PLUGIN_DIRS: &[&str] = &["platforms", "styles", "imageformats", "iconengines", "tls", "networkinformation"];

pub fn build(conf: &Conf, out: &Path, exe: &Path, arch: &str) -> Result<(), String> {
    let dir = out.join("windows-portable").join(conf.product_name());
    let copied = stage(conf, &dir, exe, arch)?;
    log("bundle", CYAN, &format!("windows-portable: {copied} DLLs, {} MiB at {}", dir_size_kib(&dir) / 1024, dir.display()));
    log("ok", GREEN, &format!("windows-portable → {}", dir.display()));
    Ok(())
}

pub fn stage(conf: &Conf, dir: &Path, exe: &Path, arch: &str) -> Result<usize, String> {
    reset_dir(dir)?;

    let staged = dir.join(exe.file_name().unwrap_or_default());
    copy_file(exe, &staged)?;
    stage_resources(conf, dir)?;

    let qt = QtPaths::query()?;
    let prune = Prune::from_conf(conf);
    let Some(process) = [&qt.libexecs, &qt.bins].into_iter().map(|dir| dir.join("QtWebEngineProcess.exe")).find(|path| path.is_file()) else {
        return Err(format!("QtWebEngineProcess.exe not found in {} or {} (install the qtwebengine module)", qt.libexecs.display(), qt.bins.display()));
    };
    copy_file(&process, &dir.join("QtWebEngineProcess.exe"))?;

    let resources = qt.data.join("resources");
    if !resources.is_dir() {
        return Err(format!("{} not found (QtWebEngine's .pak files)", resources.display()));
    }
    copy_qt(&resources, dir, true, &prune)?;

    let locales = qt.translations.join("qtwebengine_locales");
    if locales.is_dir() {
        copy_qt(&locales, &dir.join("translations/qtwebengine_locales"), true, &prune)?;
    }

    for plugin in PLUGIN_DIRS {
        let from = qt.plugins.join(plugin);
        if from.is_dir() {
            copy_qt(&from, &dir.join("plugins").join(plugin), true, &prune)?;
        }
    }
    for module in QML_MODULES {
        let from = qt.qml.join(module);
        if from.is_dir() {
            copy_qt(&from, &dir.join("qml").join(module), true, &prune)?;
        }
    }

    let mut roots = vec![staged, dir.join("QtWebEngineProcess.exe")];
    collect_dlls(&dir.join("plugins"), &mut roots);
    collect_dlls(&dir.join("qml"), &mut roots);
    let copied = stage_dlls(&roots, &qt.bins, arch, dir)?;
    check_symbols(dir, &qt.bins)?;
    Ok(copied)
}

fn check_symbols(dir: &Path, qt_bins: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect_dlls(dir, &mut files);
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    files.extend(entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))));

    let bundled = listing(dir);
    let mut exports: HashMap<String, HashSet<String>> = HashMap::new();
    let mut broken = Vec::new();
    for file in &files {
        let Ok(bytes) = std::fs::read(file) else { continue };
        let Ok(pe) = goblin::pe::PE::parse(&bytes) else { continue };
        for import in &pe.imports {
            let key = import.dll.to_ascii_lowercase();
            let Some(source) = bundled.get(&key) else { continue };
            if import.name.starts_with("ORDINAL ") {
                continue;
            }
            if !exports.entry(key).or_insert_with(|| export_names(source)).contains(import.name.as_ref()) {
                let name = file.strip_prefix(dir).unwrap_or(file).display();
                broken.push(format!("  {name} needs {} from {}, which does not export it", import.name, import.dll));
            }
        }
    }

    if broken.is_empty() {
        return Ok(());
    }
    Err(format!("the Qt DLLs in {} come from different builds, so Windows would refuse to start the app:\n{}", qt_bins.display(), broken.join("\n")))
}

fn export_names(file: &Path) -> HashSet<String> {
    let Ok(bytes) = std::fs::read(file) else { return HashSet::new() };
    match goblin::pe::PE::parse(&bytes) {
        Ok(pe) => pe.exports.iter().filter_map(|export| export.name.map(str::to_owned)).collect(),
        Err(_) => HashSet::new(),
    }
}

fn stage_dlls(roots: &[PathBuf], qt_bins: &Path, arch: &str, dest: &Path) -> Result<usize, String> {
    let qt_dlls = listing(qt_bins);
    let mut crt_dlls: Option<HashMap<String, PathBuf>> = None;

    let mut seen = BTreeSet::new();
    let mut queue = roots.to_vec();
    let mut copied = 0;

    while let Some(file) = queue.pop() {
        for import in imports(&file) {
            let key = import.to_ascii_lowercase();
            if !seen.insert(key.clone()) {
                continue;
            }
            if let Some(source) = qt_dlls.get(&key) {
                let target = dest.join(&import);
                copy_file(source, &target)?;
                queue.push(target);
                copied += 1;
            } else if key.starts_with("vcruntime") || key.starts_with("msvcp") || key.starts_with("concrt") {
                let crt = crt_dlls.get_or_insert_with(|| crt_listing(arch));
                if let Some(source) = crt.get(&key) {
                    copy_file(source, &dest.join(&import))?;
                    copied += 1;
                }
            }
        }
    }
    Ok(copied)
}

fn crt_listing(arch: &str) -> HashMap<String, PathBuf> {
    match crate::vcredist::crt_dir(arch) {
        Ok(dir) => listing(&dir),
        Err(e) => {
            log("warn", YELLOW, &format!("windows-portable: MSVC runtime not bundled ({e}), target PCs need the VC++ Redistributable"));
            HashMap::new()
        }
    }
}

pub fn imports(file: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(file) else { return Vec::new() };
    match goblin::pe::PE::parse(&bytes) {
        Ok(pe) => pe.libraries.iter().map(|lib| (*lib).to_owned()).collect(),
        Err(_) => Vec::new(),
    }
}

fn listing(dir: &Path) -> HashMap<String, PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return HashMap::new() };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("dll")))
        .map(|path| (path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase(), path))
        .collect()
}

fn collect_dlls(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dlls(&path, found);
        } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("dll")) {
            found.push(path);
        }
    }
}
