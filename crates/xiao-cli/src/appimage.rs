use crate::conf::Conf;
use crate::qt::{Prune, QML_MODULES, QtPaths, copy_qt};
use crate::util::*;
use backhand::compression::Compressor;
use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const PLUGIN_DIRS: &[&str] = &[
    "platforms",
    "platforminputcontexts",
    "wayland-decoration-client",
    "wayland-graphics-integration-client",
    "wayland-shell-integration",
    "xcbglintegrations",
    "imageformats",
    "tls",
    "networkinformation",
];
const KEEP_ON_HOST: &[&str] = &[
    "ld-linux",
    "libc.so",
    "libm.so",
    "libdl.so",
    "libpthread.so",
    "librt.so",
    "libresolv.so",
    "libnsl.so",
    "libutil.so",
    "libcrypt.so",
    "libstdc++.so",
    "libgcc_s.so",
    "libGL.so",
    "libGLX",
    "libGLdispatch",
    "libEGL.so",
    "libOpenGL.so",
    "libGLESv2",
    "libdrm",
    "libgbm",
    "libX11",
    "libxcb",
    "libXau",
    "libXdmcp",
    "libXext",
    "libXfixes",
    "libXrender",
    "libXi",
    "libXrandr",
    "libXcursor",
    "libXinerama",
    "libXcomposite",
    "libXdamage",
    "libXtst",
    "libxshmfence",
    "libwayland",
    "libgio-2.0",
    "libglib-2.0",
    "libgobject-2.0",
    "libgmodule-2.0",
    "libdbus-1",
    "libselinux",
    "libudev",
    "libsystemd",
    "libasound",
    "libpulse",
    "libfontconfig",
    "libfreetype",
];
const HOST_ENV: &[&str] = &[
    "PATH",
    "LD_LIBRARY_PATH",
    "QT_PLUGIN_PATH",
    "QML_IMPORT_PATH",
    "QML2_IMPORT_PATH",
    "QTWEBENGINEPROCESS_PATH",
    "QTWEBENGINE_RESOURCES_PATH",
    "QTWEBENGINE_LOCALES_PATH",
];
const OPENERS: &[&str] = &["xdg-open", "kde-open", "kde-open5", "gnome-open"];

pub fn build(conf: &Conf, out: &Path, arch: &str, payload: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), String> {
    let name = conf.product_name();
    let dir = out.join(format!("{name}.AppDir"));
    reset_dir(&dir)?;

    let bin_dir = dir.join("usr/bin");
    payload(&bin_dir)?;

    let qt = QtPaths::query()?;
    stage_qt_runtime(&qt, &dir, &Prune::from_conf(conf))?;
    stage_libraries(&qt, &dir, &bin_dir.join(conf.binary_name()), arch)?;

    write(&dir.join(format!("{name}.desktop")), &crate::bundle::desktop_entry(conf))?;
    let apprun = dir.join("AppRun");
    write(&apprun, &app_run(conf.binary_name()))?;
    make_executable(&apprun)?;

    for opener in OPENERS {
        let shim = dir.join("usr/shims").join(opener);
        write(&shim, &opener_shim())?;
        make_executable(&shim)?;
    }

    if let Some(icon) = conf.largest_png() {
        copy_file(&icon, &dir.join(format!("{name}.png")))?;
        copy_file(&icon, &dir.join(".DirIcon"))?;
    }

    log("bundle", CYAN, &format!("appimage: AppDir is {} MiB at {}", dir_size_kib(&dir) / 1024, dir.display()));

    let runtime = crate::tool::appimage_runtime(arch)?;
    let image = out.join(format!("{name}-{}-{arch}.AppImage", conf.version()));
    write_image(runtime, &dir, &image)?;
    log("ok", GREEN, &format!("appimage → {}", image.display()));
    Ok(())
}

fn write_image(runtime: &[u8], dir: &Path, image: &Path) -> Result<(), String> {
    let mut fs = FilesystemWriter::default();
    fs.set_compressor(FilesystemCompressor::new(Compressor::Gzip, None).map_err(|e| format!("squashfs: {e}"))?);
    fs.set_root_mode(0o755);
    fs.set_root_uid(0);
    fs.set_root_gid(0);
    add_tree(&mut fs, dir, Path::new(""))?;

    let _ = std::fs::remove_file(image);
    let mut file = std::fs::File::create(image).map_err(|e| format!("{}: {e}", image.display()))?;
    file.write_all(runtime).map_err(|e| format!("writing {}: {e}", image.display()))?;
    fs.write_with_offset(&mut file, runtime.len() as u64).map_err(|e| format!("writing the squashfs into {}: {e}", image.display()))?;
    make_executable(image)
}

fn add_tree(fs: &mut FilesystemWriter, dir: &Path, inside: &Path) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten().map(|entry| entry.path()).collect();
    entries.sort();

    for path in entries {
        let meta = path.symlink_metadata().map_err(|e| format!("{}: {e}", path.display()))?;
        let target = inside.join(path.file_name().unwrap_or_default());
        let header = NodeHeader { permissions: (meta.mode() & 0o7777) as u16, uid: 0, gid: 0, mtime: meta.mtime() as u32 };

        let pushed = if meta.is_symlink() {
            let link = std::fs::read_link(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            fs.push_symlink(link, &target, header)
        } else if meta.is_dir() {
            fs.push_dir(&target, header)
        } else {
            fs.push_file_from_path(path.clone(), &target, header)
        };
        pushed.map_err(|e| format!("squashfs: adding {}: {e}", target.display()))?;

        if meta.is_dir() {
            add_tree(fs, &path, &target)?;
        }
    }
    Ok(())
}

fn app_run(binary: &str) -> String {
    let vars = HOST_ENV.join(" ");
    format!(
        r#"#!/bin/sh
HERE=$(dirname "$(readlink -f "$0")")
for var in {vars}; do
    eval "[ \"\${{$var+set}}\" ] && export XIAO_HOST_$var=\"\$$var\""
done
export PATH="$HERE/usr/shims:$PATH"
export LD_LIBRARY_PATH="$HERE/usr/lib${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}"
export QT_PLUGIN_PATH="$HERE/usr/plugins"
export QML_IMPORT_PATH="$HERE/usr/qml"
export QML2_IMPORT_PATH="$HERE/usr/qml"
export QTWEBENGINEPROCESS_PATH="$HERE/usr/libexec/QtWebEngineProcess"
export QTWEBENGINE_RESOURCES_PATH="$HERE/usr/resources"
export QTWEBENGINE_LOCALES_PATH="$HERE/usr/translations/qtwebengine_locales"
exec "$HERE/usr/bin/{binary}" "$@"
"#
    )
}

fn opener_shim() -> String {
    let vars = HOST_ENV.join(" ");
    format!(
        r#"#!/bin/sh
for var in {vars}; do
    if eval "[ \"\${{XIAO_HOST_$var+set}}\" ]"; then
        eval "export $var=\"\$XIAO_HOST_$var\""
    else
        unset "$var"
    fi
    unset "XIAO_HOST_$var"
done
exec "${{0##*/}}" "$@"
"#
    )
}

fn stage_qt_runtime(qt: &QtPaths, dir: &Path, prune: &Prune) -> Result<(), String> {
    let process = qt.libexecs.join("QtWebEngineProcess");
    if !process.is_file() {
        return Err(format!("QtWebEngineProcess not found at {} (install qt6-webengine)", process.display()));
    }
    let staged = dir.join("usr/libexec/QtWebEngineProcess");
    copy_file(&process, &staged)?;
    make_executable(&staged)?;

    let resources = qt.data.join("resources");
    if !resources.is_dir() {
        return Err(format!("{} not found (QtWebEngine's .pak files)", resources.display()));
    }
    copy_qt(&resources, &dir.join("usr/resources"), false, prune)?;

    let locales = qt.translations.join("qtwebengine_locales");
    if locales.is_dir() {
        copy_qt(&locales, &dir.join("usr/translations/qtwebengine_locales"), false, prune)?;
    }

    for plugin in PLUGIN_DIRS {
        let from = qt.plugins.join(plugin);
        if from.is_dir() {
            copy_qt(&from, &dir.join("usr/plugins").join(plugin), false, prune)?;
        }
    }

    for module in QML_MODULES {
        let from = qt.qml.join(module);
        if from.is_dir() {
            copy_qt(&from, &dir.join("usr/qml").join(module), false, prune)?;
        }
    }
    Ok(())
}

fn stage_libraries(qt: &QtPaths, dir: &Path, binary: &Path, arch: &str) -> Result<(), String> {
    let mut roots = vec![binary.to_path_buf(), dir.join("usr/libexec/QtWebEngineProcess")];
    for staged in ["usr/plugins", "usr/qml"] {
        collect_shared_objects(&dir.join(staged), &mut roots);
    }

    let wanted: Vec<(String, PathBuf)> =
        linked_libraries(&roots, &qt.libs, arch).into_iter().filter(|(name, _)| !KEEP_ON_HOST.iter().any(|host| name.starts_with(host))).collect();

    let lib_dir = dir.join("usr/lib");
    std::fs::create_dir_all(&lib_dir).map_err(|e| format!("{}: {e}", lib_dir.display()))?;

    let mut copied = 0;
    for (name, lib) in &wanted {
        copy_file(lib, &lib_dir.join(name))?;
        copied += 1;
    }

    if !wanted.iter().any(|(name, _)| name.starts_with("libQt6WebEngineCore")) {
        return Err(format!("libQt6WebEngineCore is not linked from {}", qt.libs.display()));
    }

    log("bundle", CYAN, &format!("appimage: bundled {copied} shared libraries"));
    Ok(())
}

fn collect_shared_objects(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_shared_objects(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "so") || file_name(&path).contains(".so.") {
            found.push(path);
        }
    }
}

struct Linked {
    needed: Vec<String>,
    search: Vec<PathBuf>,
    kind: [u8; 3],
}

fn linked_libraries(roots: &[PathBuf], qt_libs: &Path, arch: &str) -> BTreeMap<String, PathBuf> {
    let system = system_lib_dirs(arch);
    let mut found = BTreeMap::new();
    let mut missing = Vec::new();
    let mut queue = roots.to_vec();

    while let Some(file) = queue.pop() {
        let Some(linked) = read_linked(&file) else { continue };
        for name in linked.needed {
            if found.contains_key(&name) || missing.contains(&name) {
                continue;
            }
            let dirs = linked.search.iter().map(PathBuf::as_path).chain([qt_libs]).chain(system.iter().map(PathBuf::as_path));
            match dirs.map(|dir| dir.join(&name)).find(|candidate| elf_kind(candidate) == Some(linked.kind)) {
                Some(path) => {
                    queue.push(path.clone());
                    found.insert(name, path);
                }
                None => missing.push(name),
            }
        }
    }

    if !missing.is_empty() {
        log("warn", YELLOW, &format!("appimage: {} not found, they must come from the host", missing.join(", ")));
    }
    found
}

fn read_linked(file: &Path) -> Option<Linked> {
    let bytes = std::fs::read(file).ok()?;
    let elf = goblin::elf::Elf::parse(&bytes).ok()?;
    let origin = file.parent().unwrap_or(Path::new("/")).to_string_lossy().into_owned();

    let paths = if elf.runpaths.is_empty() { &elf.rpaths } else { &elf.runpaths };
    let search = paths
        .iter()
        .flat_map(|list| list.split(':'))
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir.replace("${ORIGIN}", &origin).replace("$ORIGIN", &origin)))
        .collect();

    Some(Linked { needed: elf.libraries.iter().map(|lib| (*lib).to_owned()).collect(), search, kind: elf_kind_of(&bytes)? })
}

fn elf_kind(path: &Path) -> Option<[u8; 3]> {
    let mut head = [0u8; 20];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    elf_kind_of(&head)
}

fn elf_kind_of(bytes: &[u8]) -> Option<[u8; 3]> { (bytes.len() >= 20 && bytes[..4] == *b"\x7fELF").then(|| [bytes[4], bytes[18], bytes[19]]) }

fn system_lib_dirs(arch: &str) -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from(format!("/usr/lib/{arch}-linux-gnu")), PathBuf::from(format!("/lib/{arch}-linux-gnu"))];
    let mut configured = Vec::new();
    read_ld_conf(Path::new("/etc/ld.so.conf"), &mut configured, 0);

    for dir in configured.into_iter().chain(["/lib64", "/usr/lib64", "/lib", "/usr/lib"].map(PathBuf::from)) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

fn read_ld_conf(file: &Path, dirs: &mut Vec<PathBuf>, depth: u32) {
    let Ok(text) = std::fs::read_to_string(file) else { return };
    if depth > 8 {
        return;
    }

    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some(pattern) = line.strip_prefix("include ").or_else(|| line.strip_prefix("include\t")).map(str::trim) {
            let pattern = if pattern.starts_with('/') { PathBuf::from(pattern) } else { file.parent().unwrap_or(Path::new("/")).join(pattern) };
            for included in expand_glob(&pattern) {
                read_ld_conf(&included, dirs, depth + 1);
            }
        } else if line.starts_with('/') {
            let dir = PathBuf::from(line);
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
}

fn expand_glob(pattern: &Path) -> Vec<PathBuf> {
    let name = file_name(pattern);
    let Some((prefix, suffix)) = name.split_once('*') else {
        return vec![pattern.to_path_buf()];
    };
    let Some(dir) = pattern.parent() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };

    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = file_name(path);
            name.len() >= prefix.len() + suffix.len() && name.starts_with(prefix) && name.ends_with(suffix)
        })
        .collect();
    found.sort();
    found
}

fn file_name(path: &Path) -> String { path.file_name().unwrap_or_default().to_string_lossy().into_owned() }
