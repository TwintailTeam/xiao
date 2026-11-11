use crate::conf::Conf;
use crate::util::*;
#[cfg(target_os = "linux")]
use md5::Digest as _;
#[cfg(target_os = "linux")]
use std::path::Path;
use std::path::PathBuf;

pub struct Options {
    pub targets: Vec<String>,
    pub triple: Option<String>,
    pub runner: Option<String>,
    pub no_bundle: bool,
    pub debug: bool,
    pub config: Vec<String>,
}

const KNOWN: &[&str] = &["deb", "rpm", "appimage", "windows-portable", "windows-sfx", "macos-app"];

pub fn run(conf: &Conf, opts: &Options) -> Result<(), String> {
    if opts.no_bundle {
        build_release(conf, opts)?;
        log("ok", GREEN, &format!("binary → {}", release_dir(conf, opts).join(binary_file(conf, opts)).display()));
        return Ok(());
    }

    let targets = if opts.targets.is_empty() { conf.strings_at(&["bundle", "targets"]) } else { opts.targets.clone() };

    let unknown: Vec<_> = targets.iter().filter(|t| !KNOWN.contains(&t.as_str())).cloned().collect();
    if !unknown.is_empty() {
        return Err(format!("unknown target(s): {}\navailable: {}", unknown.join(", "), KNOWN.join(", ")));
    }

    let out = out_dir(conf, opts);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;

    log("bundle", CYAN, &format!("{} {} → {}", conf.product_name(), conf.version(), targets.join(", ")));

    build_release(conf, opts)?;

    if let Some(before) = conf.str_at(&["build", "beforeBundleCommand"]) {
        log("bundle", CYAN, &format!("beforeBundleCommand: {before}"));
        run_shell(before, &conf.root)?;
    }

    let os = target_os(opts);
    let arch = target_arch(opts);
    let mut failures = 0;
    for target in &targets {
        let unbuildable = match target.as_str() {
            "deb" | "rpm" | "appimage" if os != "linux" || !cfg!(target_os = "linux") => Some("only built on Linux, for Linux"),
            "windows-portable" | "windows-sfx" if os != "windows" => Some("only built for Windows (--target x86_64-pc-windows-msvc)"),
            "macos-app" if os != "macos" || !cfg!(target_os = "macos") => Some("only built on macOS, for macOS"),
            _ => None,
        };
        if let Some(reason) = unbuildable {
            log("skip", YELLOW, &format!("{target}: {reason}"));
            continue;
        }

        let result = match target.as_str() {
            #[cfg(target_os = "linux")]
            "deb" => deb(conf, opts),
            #[cfg(target_os = "linux")]
            "rpm" => rpm_package(conf, opts),
            #[cfg(target_os = "linux")]
            "appimage" => built_binary(conf, opts).and_then(|binary| crate::appimage::build(conf, &out, arch, &binary)),
            "windows-portable" => built_binary(conf, opts).and_then(|exe| crate::windows_portable::build(conf, &out, &exe, arch)),
            "windows-sfx" => windows_sfx(conf, opts),
            "macos-app" => built_binary(conf, opts).and_then(|binary| crate::macos::build(conf, &out, &binary)),
            _ => unreachable!("targets are validated above"),
        };
        if let Err(e) = result {
            failures += 1;
            log("error", RED, &format!("{target}: {e}"));
        }
    }

    if let Ok(qt) = crate::qt::QtPaths::query()
        && qt.proprietary_codecs() == Some(false)
    {
        log(
            "warn",
            YELLOW,
            &format!(
                "the QtWebEngine in {} is built without proprietary codecs: H.264, HEVC and AAC will not play, AV1, VP9, Opus, FLAC and MP3 will (in MP4 or WebM)",
                qt.libs.display()
            ),
        );
    }

    println!();
    if failures > 0 {
        return Err(format!("{failures} target(s) failed"));
    }
    log("ok", GREEN, &format!("bundles in {}", out.display()));
    Ok(())
}

fn build_release(conf: &Conf, opts: &Options) -> Result<(), String> {
    let universal = opts.triple.as_deref() == Some(UNIVERSAL);
    if universal && !cfg!(target_os = "macos") {
        return Err(format!("{UNIVERSAL} builds only on macOS"));
    }
    if target_os(opts) == "windows" && !cfg!(windows) {
        if std::env::var_os("XIAO_TARGET_QT").is_none() {
            return Err("cross-compiling for Windows: set XIAO_TARGET_QT to a Windows Qt, e.g. ~/Qt/6.12.0/msvc2022_64".to_owned());
        }
        if opts.runner.as_deref() != Some("cargo-xwin") {
            return Err("cross-compiling for Windows: pass --runner cargo-xwin".to_owned());
        }
    }
    let arch = target_arch(opts);
    if target_os(opts) == "linux" && arch != std::env::consts::ARCH && std::env::var_os("XIAO_TARGET_QT").is_none() {
        return Err(format!("cross-compiling for {arch} Linux: set XIAO_TARGET_QT to an {arch} Linux Qt, e.g. ~/Qt/6.12.0/gcc_arm64"));
    }

    if let Some(before) = conf.str_at(&["build", "beforeBuildCommand"]) {
        log("bundle", CYAN, &format!("beforeBuildCommand: {before}"));
        run_shell(before, &conf.root)?;
    }

    if universal {
        for triple in UNIVERSAL_SLICES {
            cargo_build(conf, opts, Some(triple))?;
        }
        let name = binary_file(conf, opts);
        let slices: Vec<PathBuf> = UNIVERSAL_SLICES.iter().map(|triple| conf.target_dir().join(triple).join(profile(opts)).join(&name)).collect();
        crate::fat::merge(&slices, &release_dir(conf, opts).join(&name))?;
        log("bundle", CYAN, &format!("merged {} into {UNIVERSAL}", UNIVERSAL_SLICES.join(" + ")));
    } else {
        cargo_build(conf, opts, opts.triple.as_deref())?;
    }

    stage_resources(conf, &release_dir(conf, opts))?;
    Ok(())
}

const UNIVERSAL: &str = "universal-apple-darwin";
const UNIVERSAL_SLICES: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

fn cargo_args(opts: &Options, triple: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if let Some(sub) = opts.runner.as_deref().and_then(|runner| runner.strip_prefix("cargo-")) {
        args.push(sub.to_owned());
    }
    args.push("build".to_owned());
    if !opts.debug {
        args.push("--release".to_owned());
    }
    if let Some(triple) = triple {
        args.push("--target".to_owned());
        args.push(triple.to_owned());
    }
    args
}

fn cargo_build(conf: &Conf, opts: &Options, triple: Option<&str>) -> Result<(), String> {
    let mut args = cargo_args(opts, triple);
    args.push("--workspace".to_owned());

    log("bundle", CYAN, &format!("cargo {}", args.join(" ")));

    let status = conf.cargo()?.args(&args).status().map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!("release build failed ({status})"));
    }
    Ok(())
}

fn profile(opts: &Options) -> &'static str { if opts.debug { "debug" } else { "release" } }

fn release_dir(conf: &Conf, opts: &Options) -> PathBuf {
    match &opts.triple {
        Some(triple) => conf.target_dir().join(triple).join(profile(opts)),
        None => conf.target_dir().join(profile(opts)),
    }
}

fn out_dir(conf: &Conf, opts: &Options) -> PathBuf { release_dir(conf, opts).join("bundle") }

pub fn target_os(opts: &Options) -> &'static str {
    match opts.triple.as_deref() {
        Some(triple) if triple.contains("windows") => "windows",
        Some(triple) if triple.contains("apple") => "macos",
        Some(triple) if triple.contains("linux") => "linux",
        Some(_) => "other",
        None => std::env::consts::OS,
    }
}

fn binary_file(conf: &Conf, opts: &Options) -> String {
    if target_os(opts) == "windows" { format!("{}.exe", conf.binary_name()) } else { conf.binary_name().to_owned() }
}

fn built_binary(conf: &Conf, opts: &Options) -> Result<PathBuf, String> {
    let bin = release_dir(conf, opts).join(binary_file(conf, opts));
    if !bin.is_file() {
        return Err(format!("binary not found: {}", bin.display()));
    }
    Ok(bin)
}

#[cfg(target_os = "linux")]
pub(crate) fn desktop_entry(conf: &Conf) -> String {
    let name = conf.binary_name();
    let mut lines = vec!["[Desktop Entry]".to_owned(), "Type=Application".to_owned(), format!("Name={}", conf.product_name())];
    if let Some(generic) = conf.str_at(&["bundle", "linux", "desktop", "genericName"]) {
        lines.push(format!("GenericName={generic}"));
    }
    lines.push(format!("Comment={}", conf.str_at(&["bundle", "shortDescription"]).unwrap_or("")));

    let schemes = conf.deep_link_schemes();
    if schemes.is_empty() {
        lines.push(format!("Exec={name}"));
    } else {
        lines.push(format!("Exec={name} %u"));
        lines.push(format!("MimeType={}", schemes.iter().map(|scheme| format!("x-scheme-handler/{scheme};")).collect::<String>()));
    }
    lines.push(format!("Icon={name}"));
    lines.push(format!("StartupWMClass={name}"));
    lines.push(format!("Categories={}", conf.str_at(&["bundle", "linux", "desktop", "categories"]).unwrap_or("Utility;")));
    if let Some(keywords) = conf.str_at(&["bundle", "linux", "desktop", "keywords"]) {
        lines.push(format!("Keywords={keywords}"));
    }
    lines.push("Terminal=false".to_owned());
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(target_os = "linux")]
fn stage_linux_tree(conf: &Conf, opts: &Options, stage: &Path) -> Result<(), String> {
    let name = conf.binary_name();
    let identifier = conf.identifier();
    if !identifier.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) {
        return Err(format!("`{identifier}` names folders under /usr: it may only contain letters, digits, `.`, `-` and `_`"));
    }
    reset_dir(stage)?;

    let runtime = format!("usr/libexec/{identifier}");
    let copied = crate::appimage::stage_runtime(conf, &stage.join(&runtime), &built_binary(conf, opts)?, target_arch(opts))?;
    log("bundle", CYAN, &format!("{copied} shared libraries in /{runtime}"));

    let launcher = stage.join("usr/bin").join(name);
    write(&launcher, crate::appimage::launcher_script(&format!("/{runtime}"), name))?;
    make_executable(&launcher)?;

    stage_resources(conf, &stage.join("usr/lib").join(identifier))?;

    write(&stage.join("usr/share/applications").join(format!("{name}.desktop")), desktop_entry(conf))?;

    let pngs = conf.icon_pngs();
    if pngs.is_empty() {
        log("warn", YELLOW, "no PNG in bundle.icon: the package will have no icon");
    }
    for (size, png) in pngs {
        copy_file(&png, &stage.join(format!("usr/share/icons/hicolor/{size}x{size}/apps")).join(format!("{name}.png")))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn deb(conf: &Conf, opts: &Options) -> Result<(), String> {
    let package = conf.package_name();
    let name = package.as_str();
    let version = conf.version();
    let arch = match target_arch(opts) {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    let out = out_dir(conf, opts);
    let stage = out.join("deb").join(format!("{name}_{version}_{arch}"));
    stage_linux_tree(conf, opts, &stage)?;

    let depends = conf.strings_at(&["bundle", "linux", "deb", "depends"]);
    let maintainer = conf.str_at(&["bundle", "linux", "deb", "maintainer"]).or_else(|| conf.str_at(&["bundle", "publisher"])).unwrap_or(name);
    let mut control = vec![
        format!("Package: {name}"),
        format!("Version: {version}"),
        format!("Architecture: {arch}"),
        format!("Maintainer: {maintainer}"),
        format!("Installed-Size: {}", dir_size_kib(&stage)),
    ];
    if !depends.is_empty() {
        control.push(format!("Depends: {}", depends.join(", ")));
    }
    control.push(format!("Section: {}", conf.str_at(&["bundle", "linux", "deb", "section"]).unwrap_or("utils")));
    control.push(format!("Priority: {}", conf.str_at(&["bundle", "linux", "deb", "priority"]).unwrap_or("optional")));
    if let Some(homepage) = conf.str_at(&["bundle", "homepage"]) {
        control.push(format!("Homepage: {homepage}"));
    }
    control.push(format!("Description: {}", conf.str_at(&["bundle", "shortDescription"]).unwrap_or(conf.product_name())));
    for line in conf.str_at(&["bundle", "longDescription"]).unwrap_or("").lines() {
        control.push(if line.trim().is_empty() { " .".to_owned() } else { format!(" {}", line.trim_end()) });
    }
    control.push(String::new());

    let mut sums = String::new();
    for relative in relative_files(&stage) {
        let path = stage.join(&relative);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        sums.push_str(&format!("{}  {relative}\n", hex(&md5::Md5::digest(&bytes))));
    }
    let control_tar = targz(|builder| {
        for (path, body) in [("./control", control.join("\n")), ("./md5sums", sums)] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, body.as_bytes())?;
        }
        Ok(())
    })?;

    let data_tar = targz(|builder| builder.append_dir_all(".", &stage))?;
    let deb_path = out.join(format!("{name}_{version}_{arch}.deb"));
    let _ = std::fs::remove_file(&deb_path);
    let mut archive = b"!<arch>\n".to_vec();
    for (entry, bytes) in [("debian-binary", b"2.0\n".to_vec()), ("control.tar.gz", control_tar), ("data.tar.gz", data_tar)] {
        archive.extend_from_slice(format!("{entry:<16}{:<12}{:<6}{:<6}{:<8o}{:<10}`\n", 0, 0, 0, 0o100644, bytes.len()).as_bytes());
        archive.extend_from_slice(&bytes);
        if !bytes.len().is_multiple_of(2) {
            archive.push(b'\n');
        }
    }
    write(&deb_path, archive)?;

    log("ok", GREEN, &format!("deb → {}", deb_path.display()));
    Ok(())
}

#[cfg(target_os = "linux")]
fn targz(fill: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>) -> std::io::Result<()>) -> Result<Vec<u8>, String> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.mode(tar::HeaderMode::Deterministic);
    fill(&mut builder).map_err(|e| format!("building tar: {e}"))?;
    builder.into_inner().and_then(|encoder| encoder.finish()).map_err(|e| format!("finishing tar.gz: {e}"))
}

#[cfg(target_os = "linux")]
fn rpm_package(conf: &Conf, opts: &Options) -> Result<(), String> {
    let package = conf.package_name();
    let name = package.as_str();
    let version = conf.version();
    let arch = target_arch(opts);
    let release = conf.str_at(&["bundle", "linux", "rpm", "release"]).unwrap_or("1");
    let epoch = conf.json["bundle"]["linux"]["rpm"]["epoch"].as_u64().unwrap_or(0) as u32;

    let out = out_dir(conf, opts);
    let stage = out.join("rpm").join(format!("{name}-{version}-{release}"));
    stage_linux_tree(conf, opts, &stage)?;

    let meta = crate::rpm::Metadata {
        name,
        identifier: conf.identifier(),
        version,
        release,
        epoch,
        arch,
        summary: conf.str_at(&["bundle", "shortDescription"]).unwrap_or(name),
        description: conf.str_at(&["bundle", "longDescription"]).unwrap_or(name),
        license: conf.str_at(&["bundle", "license"]).unwrap_or("Unspecified"),
        url: conf.str_at(&["bundle", "homepage"]).unwrap_or(""),
        vendor: conf.str_at(&["bundle", "publisher"]).unwrap_or(name),
        group: conf.str_at(&["bundle", "category"]).unwrap_or("Applications"),
        requires: conf.strings_at(&["bundle", "linux", "rpm", "depends"]),
    };

    let path = out.join(format!("{name}-{version}-{release}.{arch}.rpm"));
    let _ = std::fs::remove_file(&path);
    crate::rpm::write(&stage, &meta, &path)?;

    log("ok", GREEN, &format!("rpm → {}", path.display()));
    Ok(())
}

fn windows_sfx(conf: &Conf, opts: &Options) -> Result<(), String> {
    let project = crate::windows_sfx::stub_project(conf)?;
    let target_dir = project.join("target");
    let mut args = cargo_args(opts, opts.triple.as_deref());
    args.push("--target-dir".to_owned());
    args.push(target_dir.display().to_string());

    log("bundle", CYAN, &format!("cargo {} (windows-sfx launcher)", args.join(" ")));
    let status = std::process::Command::new("cargo").args(&args).current_dir(&project).status().map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!("launcher build failed ({status})"));
    }

    let built = opts.triple.as_deref().map_or_else(|| target_dir.clone(), |triple| target_dir.join(triple));
    let stub = built.join(profile(opts)).join("xiao-sfx.exe");
    crate::windows_sfx::build(conf, &out_dir(conf, opts), &stub, &built_binary(conf, opts)?, target_arch(opts))
}

fn target_arch(opts: &Options) -> &str { opts.triple.as_deref().and_then(|triple| triple.split('-').next()).unwrap_or(std::env::consts::ARCH) }
