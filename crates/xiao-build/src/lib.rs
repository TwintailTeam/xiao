use std::error::Error;
use std::path::{Path, PathBuf};

#[derive(Default, Clone)]
pub struct WindowsAttributes {
    app_manifest: Option<String>,
    icon: Option<String>,
}

impl WindowsAttributes {
    /// No overrides. Everything comes from the config file.
    pub fn new() -> Self { Self::default() }

    /// Embeds this manifest XML instead of the default one. The default is
    /// `asInvoker`, common controls v6, per-monitor DPI, long paths and UTF-8.
    pub fn app_manifest(mut self, manifest: impl Into<String>) -> Self {
        self.app_manifest = Some(manifest.into());
        self
    }

    /// The `.ico` to embed as the executable's icon, relative to the config
    /// file. Takes precedence over the `.ico` in `bundle.icon`.
    pub fn icon(mut self, path: impl Into<String>) -> Self {
        self.icon = Some(path.into());
        self
    }
}

/// The config file name. It is searched for upward from the crate's directory.
pub const CONFIG_FILE: &str = "xiao.conf.json";
const CONFIG_ENV: &str = "XIAO_CONFIG";
const QML_DIR: &str = "qml";
const RESOURCE_PREFIX: &str = "/xiao";

pub struct Attributes {
    windows: WindowsAttributes,
    qml_module: Option<String>,
    qml_entry: String,
}

impl Default for Attributes {
    fn default() -> Self { Self { windows: WindowsAttributes::new(), qml_module: None, qml_entry: "main.qml".to_owned() } }
}

impl Attributes {
    /// The defaults: QML module named after `identifier`, entry `main.qml`.
    pub fn new() -> Self { Self::default() }

    pub fn windows_attributes(mut self, windows: WindowsAttributes) -> Self {
        self.windows = windows;
        self
    }

    /// The URI of the QML module built from the crate's `qml/` directory.
    pub fn qml_module(mut self, uri: impl Into<String>) -> Self {
        self.qml_module = Some(uri.into());
        self
    }

    /// The file in `qml/` the shell loads first.
    pub fn qml_entry(mut self, file: impl Into<String>) -> Self {
        self.qml_entry = file.into();
        self
    }
}

/// [`try_build`] with default [`Attributes`]. Panics on error.
pub fn build() { try_build(Attributes::new()).unwrap_or_else(|e| panic!("xiao-build failed: {e}")); }

/// Run from the app's `build.rs`. It reads [`CONFIG_FILE`], builds the
/// `qml/` module, embeds the frontend and icons, and writes `config.rs` to
/// `OUT_DIR` with `CONFIG_JSON` and `QML_ENTRY`. On Windows it also embeds
/// the manifest and icon.
///
/// `xiao.<os>.conf.json` next to the config (`linux`, `windows`, `macos`) and
/// then the `XIAO_CONFIG` variable are merged over it as JSON Merge Patches.
///
/// A release build fails if `build.frontendDist` has no `index.html`.
pub fn try_build(attrs: Attributes) -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let config_path = manifest_dir
        .ancestors()
        .map(|dir| dir.join(CONFIG_FILE))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("cannot find {CONFIG_FILE} in {} or any parent directory", manifest_dir.display()))?;

    let mut config = read_json(&config_path)?;

    let platform = config_path.with_file_name(format!("xiao.{}.conf.json", std::env::var("CARGO_CFG_TARGET_OS")?));
    if platform.is_file() {
        merge(&mut config, read_json(&platform)?);
    }

    println!("cargo:rerun-if-env-changed={CONFIG_ENV}");
    if let Ok(extra) = std::env::var(CONFIG_ENV) {
        match serde_json::from_str(&extra).map_err(|e| format!("{CONFIG_ENV} is not valid JSON: {e}"))? {
            serde_json::Value::Array(patches) => patches.into_iter().for_each(|patch| merge(&mut config, patch)),
            patch => merge(&mut config, patch),
        }
    }

    for required in ["productName", "version", "identifier"] {
        let ok = config.get(required).and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty());
        if !ok {
            return Err(format!("{}: `{required}` must be a non-empty string", config_path.display()).into());
        }
    }

    let mut unknown = Vec::new();
    unknown_keys(&config, "", &mut unknown);
    for key in unknown {
        println!("cargo:warning={CONFIG_FILE}: `{key}` is not a key xiao reads, check its spelling");
    }

    let module_uri =
        attrs.qml_module.clone().or_else(|| config["identifier"].as_str().map(qml_uri)).ok_or("`identifier` is required to name the app's QML module")?;
    select_linker();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path/../Frameworks");
    }
    write_config(&out_dir, &config, &module_uri, &attrs.qml_entry)?;
    embed_windows_manifest(&config_path, &config, &attrs.windows)?;
    embed_windows_icon(&config_path, &config, &attrs.windows)?;
    build_qml_module(&manifest_dir, &module_uri, embedded_assets(&config_path, &config)?)?;
    Ok(())
}

fn read_json(path: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
    println!("cargo:rerun-if-changed={}", path.display());
    let raw = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(serde_json::from_str(&raw).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?)
}

fn merge(base: &mut serde_json::Value, patch: serde_json::Value) {
    let serde_json::Value::Object(entries) = patch else {
        *base = patch;
        return;
    };
    if !base.is_object() {
        *base = serde_json::Value::Object(serde_json::Map::new());
    }
    let serde_json::Value::Object(target) = base else { return };
    for (key, value) in entries {
        if value.is_null() {
            target.remove(&key);
        } else {
            merge(target.entry(key).or_insert(serde_json::Value::Null), value);
        }
    }
}

const KNOWN_KEYS: &[&str] = &[
    "$schema",
    "productName",
    "mainBinaryName",
    "version",
    "identifier",
    "build.appDir",
    "build.beforeDevCommand",
    "build.devUrl",
    "build.beforeBuildCommand",
    "build.beforeBundleCommand",
    "build.frontendDist",
    "build.additionalWatchFolders",
    "app.singleInstance",
    "app.webEngine.matchGpu",
    "app.deepLink.schemes",
    "app.trayIcon.iconPath",
    "app.trayIcon.tooltip",
    "app.windows[].title",
    "app.windows[].width",
    "app.windows[].height",
    "app.windows[].minWidth",
    "app.windows[].minHeight",
    "app.windows[].maxWidth",
    "app.windows[].maxHeight",
    "app.windows[].x",
    "app.windows[].y",
    "app.windows[].resizable",
    "app.windows[].decorations",
    "app.windows[].alwaysOnTop",
    "app.windows[].center",
    "app.windows[].fullscreen",
    "app.windows[].maximized",
    "app.windows[].visible",
    "app.windows[].backgroundColor",
    "app.windows[].whenHidden",
    "app.security.page.readClipboard",
    "app.security.page.writeClipboard",
    "app.security.page.fileDialogs",
    "app.security.page.createDirectories",
    "app.security.page.registerDeepLinks",
    "app.security.page.downloads",
    "app.security.opener.schemes",
    "app.security.opener.paths",
    "app.security.csp",
    "app.security.devCsp",
    "bundle.targets",
    "bundle.icon",
    "bundle.resources",
    "bundle.webengineLocales",
    "bundle.category",
    "bundle.shortDescription",
    "bundle.longDescription",
    "bundle.publisher",
    "bundle.homepage",
    "bundle.license",
    "bundle.linux.desktop.categories",
    "bundle.linux.desktop.genericName",
    "bundle.linux.desktop.keywords",
    "bundle.linux.deb.section",
    "bundle.linux.deb.priority",
    "bundle.linux.deb.maintainer",
    "bundle.linux.deb.depends",
    "bundle.linux.rpm.release",
    "bundle.linux.rpm.epoch",
    "bundle.linux.rpm.depends",
    "bundle.macOS.minimumSystemVersion",
    "bundle.macOS.category",
    "bundle.windows.appManifest",
    "bundle.windows.appManifestPath",
];

const FREE_FORM_KEYS: &[&str] = &["app.security.csp", "app.security.devCsp"];

fn unknown_keys(value: &serde_json::Value, path: &str, found: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(entries) => {
            for (key, child) in entries {
                let here = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                let known = KNOWN_KEYS
                    .iter()
                    .any(|known| known.strip_prefix(here.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with('.') || rest.starts_with("[]")));
                if !known {
                    found.push(here);
                } else if !FREE_FORM_KEYS.contains(&here.as_str()) {
                    unknown_keys(child, &here, found);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                unknown_keys(item, &format!("{path}[]"), found);
            }
        }
        _ => {}
    }
}

fn embedded_assets(config_path: &Path, config: &serde_json::Value) -> Result<cxx_qt_build::QResources, Box<dyn Error>> {
    let root = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut resource = cxx_qt_build::QResource::new().prefix(RESOURCE_PREFIX);

    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let frontend = root.join(config["build"]["frontendDist"].as_str().unwrap_or("dist"));
    let mut files = Vec::new();
    if frontend.is_dir() {
        println!("cargo:rerun-if-changed={}", frontend.display());
        collect_files(&frontend, &frontend, &mut files);
    }
    if !files.iter().any(|(_, relative)| relative == "index.html") {
        let problem = format!("{} has no index.html: build the frontend first", frontend.display());
        if release {
            return Err(problem.into());
        }
        println!("cargo:warning={} has no index.html: this debug build will only load the dev server", frontend.display());
    }
    for (path, relative) in files {
        println!("cargo:rerun-if-changed={}", path.display());
        resource = resource.file(qt_build_utils::QResourceFile::new(path).alias(format!("dist/{relative}")));
    }

    let listed: Vec<String> = match &config["bundle"]["icon"] {
        serde_json::Value::Array(items) => items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect(),
        serde_json::Value::String(one) => vec![one.clone()],
        _ => Vec::new(),
    };
    let mut icons: Vec<String> = listed.into_iter().filter(|icon| icon.ends_with(".png")).collect();
    if let Some(tray) = config["app"]["trayIcon"]["iconPath"].as_str().filter(|icon| !icon.is_empty()) {
        icons.push(tray.to_owned());
    }

    let mut aliases = std::collections::BTreeSet::new();
    for icon in icons {
        let path = root.join(&icon);
        if !path.is_file() {
            println!("cargo:warning={icon} is listed as an icon but does not exist");
            continue;
        }
        let alias = format!("icons/{}", file_name(&icon));
        if !aliases.insert(alias.clone()) {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        resource = resource.file(qt_build_utils::QResourceFile::new(path).alias(alias));
    }

    Ok(cxx_qt_build::QResources::new().resource(resource))
}

fn qml_uri(identifier: &str) -> String {
    identifier.split('.').map(|part| part.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>()).collect::<Vec<_>>().join(".")
}

fn file_name(path: &str) -> &str { path.rsplit(['/', '\\']).next().unwrap_or(path) }

fn collect_files(base: &Path, dir: &Path, found: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, found);
        } else if let Ok(relative) = path.strip_prefix(base) {
            let relative = relative.components().map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            found.push((path.clone(), relative));
        }
    }
    found.sort();
}

fn build_qml_module(manifest_dir: &Path, module_uri: &str, assets: cxx_qt_build::QResources) -> Result<(), Box<dyn Error>> {
    let qml_dir = manifest_dir.join(QML_DIR);
    if !qml_dir.is_dir() {
        return Err(format!("no {QML_DIR}/ directory in {}", manifest_dir.display()).into());
    }

    let mut files: Vec<String> = std::fs::read_dir(&qml_dir)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "qml"))
        .map(|path| format!("{QML_DIR}/{}", path.file_name().unwrap_or_default().display()))
        .collect();
    files.sort();

    if files.is_empty() {
        return Err(format!("{} contains no .qml files", qml_dir.display()).into());
    }

    println!("cargo:rerun-if-changed={QML_DIR}");
    let mut module = cxx_qt_build::QmlModule::new(module_uri);
    for file in &files {
        println!("cargo:rerun-if-changed={file}");
        module = module.qml_file(file);
    }

    cxx_qt_build::CxxQtBuilder::new_qml_module(module).qrc_resources(assets).build();
    Ok(())
}

fn select_linker() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        return;
    }

    let requested = std::env::var("XIAO_LINKER");
    if requested.as_deref() == Ok("") {
        return;
    }

    let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    if flags.contains("fuse-ld") || flags.contains("linker=") {
        return;
    }

    let candidates: Vec<String> = match requested {
        Ok(name) => vec![name],
        Err(_) => vec!["mold".to_owned(), "lld".to_owned()],
    };

    let on_path = |program: &String| std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()));
    match candidates.into_iter().find(on_path) {
        Some(linker) => println!("cargo:rustc-link-arg=-fuse-ld={linker}"),
        None => println!("cargo:warning=neither mold nor lld found: linking may fail with GNU ld (install one, or set XIAO_LINKER)"),
    }
}

fn write_config(out_dir: &Path, config: &serde_json::Value, module_uri: &str, qml_entry: &str) -> Result<(), Box<dyn Error>> {
    let compact = serde_json::to_string(config)?;

    let mut hashes = String::from("##");
    while compact.contains(&format!("\"{hashes}")) {
        hashes.push('#');
    }
    let qml_url = format!("qrc:/qt/qml/{}/{QML_DIR}/{qml_entry}", module_uri.replace('.', "/"));

    let out = out_dir.join("config.rs");
    std::fs::write(
        &out,
        format!(
            "pub const CONFIG_JSON: &str = r{hashes}\"{compact}\"{hashes};\n\
             pub const QML_ENTRY: &str = \"{qml_url}\";\n"
        ),
    )
    .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    Ok(())
}

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;

fn icon_res(ico: &[u8]) -> Result<Vec<u8>, String> {
    let u16_at = |at: usize| ico.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or("truncated .ico");
    let u32_at = |at: usize| ico.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or("truncated .ico");

    if u16_at(0)? != 0 || u16_at(2)? != 1 {
        return Err("not an .ico file".to_owned());
    }
    let count = u16_at(4)? as usize;
    if count == 0 {
        return Err("the .ico holds no images".to_owned());
    }

    let mut res = Vec::new();
    res_entry(&mut res, 0, 0, 0, &[]);

    let mut group = Vec::with_capacity(6 + count * 14);
    group.extend_from_slice(&[0, 0, 1, 0]);
    group.extend_from_slice(&(count as u16).to_le_bytes());

    for index in 0..count {
        let entry = 6 + index * 16;
        let size = u32_at(entry + 8)? as usize;
        let offset = u32_at(entry + 12)? as usize;
        let image = ico.get(offset..offset + size).ok_or("an image runs past the end of the .ico")?;
        let id = index as u16 + 1;

        res_entry(&mut res, RT_ICON, id, 0x1010, image);

        group.extend_from_slice(&ico[entry..entry + 8]);
        group.extend_from_slice(&(size as u32).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }

    res_entry(&mut res, RT_GROUP_ICON, 1, 0x1030, &group);
    Ok(res)
}

fn res_entry(res: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    res.extend_from_slice(&(data.len() as u32).to_le_bytes());
    res.extend_from_slice(&32u32.to_le_bytes());
    for ordinal in [kind, name] {
        res.extend_from_slice(&0xFFFFu16.to_le_bytes());
        res.extend_from_slice(&ordinal.to_le_bytes());
    }
    res.extend_from_slice(&0u32.to_le_bytes());
    res.extend_from_slice(&flags.to_le_bytes());
    res.extend_from_slice(&(if kind == 0 { 0u16 } else { 0x0409 }).to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes());
    res.extend_from_slice(data);
    while !res.len().is_multiple_of(4) {
        res.push(0);
    }
}

fn embed_windows_manifest(config_path: &Path, config: &serde_json::Value, windows: &WindowsAttributes) -> Result<(), Box<dyn Error>> {
    let win_cfg = &config["bundle"]["windows"];

    let manifest = match &windows.app_manifest {
        Some(xml) => Some(xml.clone()),
        None => match win_cfg.get("appManifest").and_then(|v| v.as_str()) {
            Some(xml) => Some(xml.to_owned()),
            None => match win_cfg.get("appManifestPath").and_then(|v| v.as_str()) {
                Some(rel) => {
                    let base = config_path.parent().unwrap_or(Path::new("."));
                    let path = base.join(rel);
                    println!("cargo:rerun-if-changed={}", path.display());
                    Some(std::fs::read_to_string(&path).map_err(|e| format!("cannot read manifest {}: {e}", path.display()))?)
                }
                None => None,
            },
        },
    };

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let path = out_dir.join("app.manifest");
    std::fs::write(&path, manifest.as_deref().unwrap_or(DEFAULT_MANIFEST)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", path.display());
    } else {
        println!(
            "cargo:warning=app manifest written to {} but not embedded: only the MSVC toolchain is supported (use x86_64-pc-windows-msvc)",
            path.display()
        );
    }
    Ok(())
}

const DEFAULT_MANIFEST: &str = r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">permonitorv2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
</assembly>
"#;

fn embed_windows_icon(config_path: &Path, config: &serde_json::Value, windows: &WindowsAttributes) -> Result<(), Box<dyn Error>> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }

    let from_bundle = || match &config["bundle"]["icon"] {
        serde_json::Value::Array(items) => items.iter().filter_map(|v| v.as_str()).find(|p| p.ends_with(".ico")).map(String::from),
        serde_json::Value::String(one) if one.ends_with(".ico") => Some(one.clone()),
        _ => None,
    };

    let Some(relative) = windows.icon.clone().or_else(from_bundle) else {
        println!("cargo:warning=no .ico in bundle.icon: the exe will use the default icon");
        return Ok(());
    };

    let ico = config_path.parent().unwrap_or(Path::new(".")).join(&relative);
    if !ico.is_file() {
        println!("cargo:warning=icon {} not found: the exe will use the default icon", ico.display());
        return Ok(());
    }
    println!("cargo:rerun-if-changed={}", ico.display());

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        println!("cargo:warning=icon {} not embedded: only the MSVC toolchain is supported (use x86_64-pc-windows-msvc)", ico.display());
        return Ok(());
    }

    let ico_bytes = std::fs::read(&ico).map_err(|e| format!("cannot read {}: {e}", ico.display()))?;
    let res = icon_res(&ico_bytes).map_err(|e| format!("{}: {e}", ico.display()))?;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let res_path = out_dir.join("icon.res");
    std::fs::write(&res_path, res).map_err(|e| format!("cannot write {}: {e}", res_path.display()))?;
    println!("cargo:rustc-link-arg-bins={}", res_path.display());
    Ok(())
}
