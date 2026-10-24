use crate::conf::Conf;
use crate::util::*;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"XIAOSFX1";

const SOURCES: &[(&str, &str)] = &[
    ("Cargo.toml", include_str!("../../xiao-sfx/Cargo.toml")),
    ("build.rs", include_str!("../../xiao-sfx/build.rs")),
    (".cargo/config.toml", include_str!("../../xiao-sfx/.cargo/config.toml")),
    ("src/main.rs", include_str!("../../xiao-sfx/src/main.rs")),
    ("src/inflate.rs", include_str!("../../xiao-sfx/src/inflate.rs")),
];

const SYSTEM_DLLS: &[&str] = &["kernel32.dll", "user32.dll", "ntdll.dll", "advapi32.dll", "bcryptprimitives.dll", "ws2_32.dll", "userenv.dll"];

const MANIFEST: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" />
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
    </windowsSettings>
  </application>
</assembly>
"#;

pub fn stub_project(conf: &Conf) -> Result<PathBuf, String> {
    let dir = conf.target_dir().join("xiao-sfx");
    for (path, contents) in SOURCES {
        write_if_changed(&dir.join(path), contents.as_bytes())?;
    }
    write_if_changed(&dir.join("app.manifest"), MANIFEST.as_bytes())?;

    let icon = dir.join("icon.res");
    match conf.icons().into_iter().find(|icon| icon.ends_with(".ico")) {
        Some(relative) => {
            let ico = conf.asset(&relative);
            let bytes = std::fs::read(&ico).map_err(|e| format!("{}: {e}", ico.display()))?;
            write_if_changed(&icon, &icon_res(&bytes).map_err(|e| format!("{}: {e}", ico.display()))?)?;
        }
        None => {
            let _ = std::fs::remove_file(&icon);
        }
    }
    Ok(dir)
}

pub fn build(conf: &Conf, out: &Path, stub: &Path, exe: &Path, arch: &str) -> Result<(), String> {
    let foreign: Vec<String> = crate::windows_portable::imports(stub)
        .into_iter()
        .filter(|dll| {
            let dll = dll.to_ascii_lowercase();
            !SYSTEM_DLLS.contains(&dll.as_str()) && !dll.starts_with("api-ms-win-core-")
        })
        .collect();
    if !foreign.is_empty() {
        return Err(format!("{} imports {}, but it must run with nothing beside it", stub.display(), foreign.join(", ")));
    }
    for part in [conf.identifier(), conf.version()] {
        if part.contains(['/', '\\', ':']) || part == "." || part == ".." {
            return Err(format!("`{part}` is used as a folder name on Windows and cannot contain / \\ :"));
        }
    }

    let name = conf.product_name();
    let root = out.join("windows-sfx");
    let stage = root.join(name);
    let copied = crate::windows_portable::stage(conf, &stage, exe, arch)?;

    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    let target = root.join(format!("{name}_{}_{arch}.exe", conf.version()));
    let exe_name = exe.file_name().unwrap_or_default().to_string_lossy();
    log("bundle", CYAN, &format!("windows-sfx: compressing {copied} DLLs, {} MiB", dir_size_kib(&stage) / 1024));
    pack(conf, stub, &stage, &exe_name, &target).map_err(|e| format!("{}: {e}", target.display()))?;

    let size = std::fs::metadata(&target).map(|meta| meta.len() / (1024 * 1024)).unwrap_or(0);
    log("ok", GREEN, &format!("windows-sfx → {} ({size} MiB)", target.display()));
    Ok(())
}

fn pack(conf: &Conf, stub: &Path, stage: &Path, exe: &str, target: &Path) -> Result<(), String> {
    let bytes = std::fs::read(stub).map_err(|e| format!("{}: {e}", stub.display()))?;
    let pe = goblin::pe::PE::parse(&bytes).map_err(|e| format!("{}: {e}", stub.display()))?;
    let end = pe.sections.iter().filter(|s| s.pointer_to_raw_data > 0).map(|s| s.pointer_to_raw_data as usize + s.size_of_raw_data as usize).max();
    if end != Some(bytes.len()) {
        return Err(format!("{} has data after its last section", stub.display()));
    }

    let mut files = Vec::new();
    walk(stage, stage, &mut files);
    files.sort();

    let io = |e: std::io::Error| e.to_string();
    let mut out = BufWriter::new(File::create(target).map_err(io)?);
    out.write_all(&bytes).map_err(io)?;
    out.write_all(MAGIC).map_err(io)?;
    out.write_all(&[0; 8 + 6]).map_err(io)?;
    for text in [conf.identifier(), conf.version(), exe] {
        out.write_all(&(text.len() as u16).to_le_bytes()).map_err(io)?;
        out.write_all(text.as_bytes()).map_err(io)?;
    }

    let mut encoder = flate2::write::ZlibEncoder::new(Hashed { out, hasher: Sha256::new(), written: 0 }, flate2::Compression::default());
    for relative in &files {
        let path = stage.join(relative);
        let mut source = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let size = source.metadata().map_err(io)?.len();
        encoder.write_all(&(relative.len() as u16).to_le_bytes()).map_err(io)?;
        encoder.write_all(relative.as_bytes()).map_err(io)?;
        encoder.write_all(&size.to_le_bytes()).map_err(io)?;
        if std::io::copy(&mut source, &mut encoder).map_err(io)? != size {
            return Err(format!("{} changed while it was being packed", path.display()));
        }
    }
    encoder.write_all(&0u16.to_le_bytes()).map_err(io)?;
    let hashed = encoder.finish().map_err(io)?;

    let mut file = hashed.out.into_inner().map_err(|e| e.to_string())?;
    let key = hashed.hasher.finalize();
    file.seek(SeekFrom::Start(bytes.len() as u64 + MAGIC.len() as u64)).map_err(io)?;
    file.write_all(&hashed.written.to_le_bytes()).map_err(io)?;
    file.write_all(&key[..6]).map_err(io)?;
    Ok(())
}

struct Hashed {
    out: BufWriter<File>,
    hasher: Sha256,
    written: u64,
}

impl Write for Hashed {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let count = self.out.write(buf)?;
        self.hasher.update(&buf[..count]);
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> { self.out.flush() }
}

fn walk(base: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(base, &path, found);
        } else if let Ok(relative) = path.strip_prefix(base) {
            found.push(relative.components().map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
        }
    }
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<(), String> {
    if std::fs::read(path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, contents).map_err(|e| format!("writing {}: {e}", path.display()))
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
