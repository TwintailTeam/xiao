use crate::conf::Conf;
use crate::qt::{Prune, QML_MODULES, QtPaths, copy_qt};
use crate::util::*;
use goblin::mach::{Mach, MachO, SingleArch};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PLUGIN_DIRS: &[&str] = &["platforms", "styles", "imageformats", "iconengines", "tls", "networkinformation", "permissions"];

pub fn build(conf: &Conf, out: &Path, binary: &Path) -> Result<(), String> {
    let name = conf.product_name();
    let app_dir = out.join(format!("{name}.app"));
    reset_dir(&app_dir)?;

    let contents = app_dir.join("Contents");
    let executable = contents.join("MacOS").join(conf.binary_name());
    copy_file(binary, &executable)?;
    make_executable(&executable)?;

    let resources = contents.join("Resources");
    stage_resources(conf, &resources)?;

    let icns = build_icns(&conf.icon_pngs(), &resources.join("icon.icns"))?;
    if !icns {
        log("warn", YELLOW, "macos-app: no PNG in bundle.icon: the bundle will have no icon");
    }
    write(&contents.join("Info.plist"), &info_plist(conf, icns))?;

    let qt = QtPaths::query()?;
    let prune = Prune::from_conf(conf);
    for plugin in PLUGIN_DIRS {
        let from = qt.plugins.join(plugin);
        if from.is_dir() {
            copy_qt(&from, &contents.join("PlugIns").join(plugin), true, &prune)?;
        }
    }
    for module in QML_MODULES {
        let from = qt.qml.join(module);
        if from.is_dir() {
            copy_qt(&from, &resources.join("qml").join(module), true, &prune)?;
        }
    }

    let mut roots = vec![executable];
    collect_dylibs(&contents.join("PlugIns"), &mut roots);
    collect_dylibs(&resources.join("qml"), &mut roots);
    let copied = stage_frameworks(&roots, &qt.libs, &contents.join("Frameworks"), &prune)?;

    log("bundle", CYAN, &format!("macos-app: {copied} frameworks and libraries, {} MiB", dir_size_kib(&app_dir) / 1024));
    log("ok", GREEN, &format!("macos-app → {}", app_dir.display()));
    Ok(())
}

fn stage_frameworks(roots: &[PathBuf], qt_libs: &Path, frameworks: &Path, prune: &Prune) -> Result<usize, String> {
    let mut seen = BTreeSet::new();
    let mut queue = roots.to_vec();
    let mut copied = 0;

    while let Some(file) = queue.pop() {
        for lib in load_commands(&file) {
            let Some(relative) = lib.strip_prefix("@rpath/") else {
                if lib.starts_with('/') && !lib.starts_with("/System/") && !lib.starts_with("/usr/lib/") {
                    return Err(format!("{} loads {lib} by absolute path (Homebrew Qt), use Qt from the official installer or aqtinstall", file.display()));
                }
                continue;
            };

            let top = relative.split('/').next().unwrap_or(relative);
            if !seen.insert(top.to_owned()) {
                continue;
            }

            let source = qt_libs.join(top);
            let target = frameworks.join(top);
            if top.ends_with(".framework") && source.is_dir() {
                copy_framework(&source, &target, prune)?;
                collect_helpers(&target, &mut queue);
            } else if source.is_file() {
                copy_file(&source, &target)?;
            } else {
                log("warn", YELLOW, &format!("macos-app: {lib} is not in {}, it must come from the system", qt_libs.display()));
                continue;
            }
            queue.push(frameworks.join(relative));
            copied += 1;
        }
    }
    Ok(copied)
}

fn load_commands(file: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(file) else { return Vec::new() };
    let libs = |macho: MachO| macho.libs.iter().filter(|lib| **lib != "self").map(|lib| (*lib).to_owned()).collect();
    match Mach::parse(&bytes) {
        Ok(Mach::Binary(macho)) => libs(macho),
        Ok(Mach::Fat(fat)) => match fat.get(0) {
            Ok(SingleArch::MachO(macho)) => libs(macho),
            _ => Vec::new(),
        },
        Err(_) => Vec::new(),
    }
}

fn copy_framework(from: &Path, to: &Path, prune: &Prune) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;

    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "Headers" || name.ends_with(".dSYM") || name.ends_with("_debug") || name.contains("_debug.") || name.ends_with(".prl") {
            continue;
        }

        let src = entry.path();
        let dest = to.join(&name);
        let meta = src.symlink_metadata().map_err(|e| format!("{}: {e}", src.display()))?;
        if meta.is_symlink() {
            let link = std::fs::read_link(&src).map_err(|e| format!("{}: {e}", src.display()))?;
            symlink(&link.to_string_lossy(), &dest)?;
        } else if meta.is_dir() {
            copy_framework(&src, &dest, prune)?;
        } else if prune.drops_webengine_file(&src) {
            continue;
        } else {
            copy_file(&src, &dest)?;
        }
    }
    Ok(())
}

fn collect_helpers(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = path.symlink_metadata() else { continue };
        if meta.is_dir() {
            collect_helpers(&path, found);
        } else if meta.is_file() && path.parent().is_some_and(|parent| parent.ends_with("Contents/MacOS")) {
            found.push(path);
        }
    }
}

fn collect_dylibs(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dylibs(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "dylib") {
            found.push(path);
        }
    }
}

fn info_plist(conf: &Conf, icns: bool) -> String {
    let icon_key = if icns { "  <key>CFBundleIconFile</key><string>icon.icns</string>\n" } else { "" };

    let schemes = conf.deep_link_schemes();
    let url_types = if schemes.is_empty() {
        String::new()
    } else {
        let listed: String = schemes.iter().map(|scheme| format!("      <string>{scheme}</string>\n")).collect();
        format!(
            "  <key>CFBundleURLTypes</key>\n  <array>\n    <dict>\n      <key>CFBundleURLName</key><string>{}</string>\n      <key>CFBundleURLSchemes</key>\n      <array>\n{listed}      </array>\n    </dict>\n  </array>\n",
            conf.identifier()
        )
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>{name}</string>
  <key>CFBundleDisplayName</key><string>{name}</string>
  <key>CFBundleIdentifier</key><string>{identifier}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleExecutable</key><string>{binary}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
{icon_key}  <key>LSMinimumSystemVersion</key><string>{min_version}</string>
  <key>LSApplicationCategoryType</key><string>{category}</string>
  <key>NSHighResolutionCapable</key><true/>
{url_types}</dict>
</plist>
"#,
        name = conf.product_name(),
        identifier = conf.identifier(),
        version = conf.version(),
        binary = conf.binary_name(),
        min_version = conf.str_at(&["bundle", "macOS", "minimumSystemVersion"]).unwrap_or("12.0"),
        category = conf.str_at(&["bundle", "macOS", "category"]).unwrap_or("public.app-category.utilities"),
    )
}

fn build_icns(icons: &[(u32, PathBuf)], dest: &Path) -> Result<bool, String> {
    const TYPES: &[(&str, u32)] =
        &[("icp4", 16), ("icp5", 32), ("ic11", 32), ("ic12", 64), ("ic07", 128), ("ic08", 256), ("ic13", 256), ("ic09", 512), ("ic14", 512)];

    let mut body: Vec<u8> = Vec::new();
    for (ostype, size) in TYPES {
        let Some((_, file)) = icons.iter().find(|(s, _)| s == size) else {
            continue;
        };
        let png = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
        body.extend_from_slice(ostype.as_bytes());
        body.extend_from_slice(&((png.len() + 8) as u32).to_be_bytes());
        body.extend_from_slice(&png);
    }
    if body.is_empty() {
        return Ok(false);
    }

    let mut icns = Vec::with_capacity(body.len() + 8);
    icns.extend_from_slice(b"icns");
    icns.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    icns.extend_from_slice(&body);

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(dest, icns).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(true)
}
