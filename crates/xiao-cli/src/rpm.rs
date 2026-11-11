use crate::util::hex;
use md5::Digest as _;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const TYPE_INT16: u32 = 3;
const TYPE_INT32: u32 = 4;
const TYPE_STRING: u32 = 6;
const TYPE_BIN: u32 = 7;
const TYPE_STRING_ARRAY: u32 = 8;
const TYPE_I18NSTRING: u32 = 9;

const HEADER_MAGIC: [u8; 8] = [0x8e, 0xad, 0xe8, 0x01, 0, 0, 0, 0];
const REGION_SIGNATURES: u32 = 62;
const REGION_IMMUTABLE: u32 = 63;

const RPMLIB_SENSE: u32 = (1 << 24) | 8 | 2;
const SENSE_EQUAL: u32 = 8;
const FILEDIGEST_SHA256: u32 = 8;

pub struct Metadata<'a> {
    pub name: &'a str,
    pub identifier: &'a str,
    pub version: &'a str,
    pub release: &'a str,
    pub epoch: u32,
    pub arch: &'a str,
    pub summary: &'a str,
    pub description: &'a str,
    pub license: &'a str,
    pub url: &'a str,
    pub vendor: &'a str,
    pub group: &'a str,
    pub requires: Vec<String>,
}

enum Value {
    Int16(Vec<u16>),
    Int32(Vec<u32>),
    Str(String),
    I18n(String),
    StrArray(Vec<String>),
    Bin(Vec<u8>),
}

fn header(region: u32, mut entries: Vec<(u32, Value)>) -> Vec<u8> {
    entries.sort_by_key(|(tag, _)| *tag);

    let mut store: Vec<u8> = Vec::new();
    let mut index: Vec<[u32; 4]> = Vec::new();

    for (tag, value) in &entries {
        let align = match value {
            Value::Int16(_) => 2,
            Value::Int32(_) => 4,
            _ => 1,
        };
        while !store.len().is_multiple_of(align) {
            store.push(0);
        }

        let offset = store.len() as u32;
        let (kind, count) = match value {
            Value::Int16(items) => {
                for item in items {
                    store.extend_from_slice(&item.to_be_bytes());
                }
                (TYPE_INT16, items.len() as u32)
            }
            Value::Int32(items) => {
                for item in items {
                    store.extend_from_slice(&item.to_be_bytes());
                }
                (TYPE_INT32, items.len() as u32)
            }
            Value::Str(text) | Value::I18n(text) => {
                store.extend_from_slice(text.as_bytes());
                store.push(0);
                let kind = if matches!(value, Value::Str(_)) { TYPE_STRING } else { TYPE_I18NSTRING };
                (kind, 1)
            }
            Value::StrArray(items) => {
                for item in items {
                    store.extend_from_slice(item.as_bytes());
                    store.push(0);
                }
                (TYPE_STRING_ARRAY, items.len() as u32)
            }
            Value::Bin(bytes) => {
                store.extend_from_slice(bytes);
                (TYPE_BIN, bytes.len() as u32)
            }
        };

        index.push([*tag, kind, offset, count]);
    }

    let trailer_offset = store.len() as u32;
    let total_entries = index.len() as u32 + 1;
    let back = -((total_entries as i32) * 16) as u32;
    for field in [region, TYPE_BIN, back, 16u32] {
        store.extend_from_slice(&field.to_be_bytes());
    }

    let mut out = Vec::with_capacity(16 + total_entries as usize * 16 + store.len());
    out.extend_from_slice(&HEADER_MAGIC);
    out.extend_from_slice(&total_entries.to_be_bytes());
    out.extend_from_slice(&(store.len() as u32).to_be_bytes());

    for field in [region, TYPE_BIN, trailer_offset, 16u32] {
        out.extend_from_slice(&field.to_be_bytes());
    }
    for fields in &index {
        for field in fields {
            out.extend_from_slice(&field.to_be_bytes());
        }
    }
    out.extend_from_slice(&store);
    out
}

fn lead(meta: &Metadata) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&[0xed, 0xab, 0xee, 0xdb]);
    out.push(3);
    out.push(0);
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());

    let mut name = format!("{}-{}-{}", meta.name, meta.version, meta.release).into_bytes();
    name.resize(66, 0);
    out.extend_from_slice(&name);

    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&5u16.to_be_bytes());
    out.extend_from_slice(&[0u8; 16]);
    out
}

struct Payload {
    cpio: Vec<u8>,
    files: Vec<File>,
    total_size: u64,
}

struct File {
    installed: String,
    size: u32,
    mode: u16,
    mtime: u32,
    digest: String,
    link_to: String,
}

fn cpio_entry(out: &mut Vec<u8>, name: &str, mode: u32, size: u32, mtime: u32, inode: u32) {
    let fields = [inode, mode, 0, 0, 1, mtime, size, 0, 0, 0, 0, name.len() as u32 + 1, 0];

    out.extend_from_slice(b"070701");
    for field in fields {
        out.extend_from_slice(format!("{field:08X}").as_bytes());
    }
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn collect(stage: &Path, identifier: &str) -> Result<Payload, String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    walk(stage, &mut paths);
    paths.sort();

    let owned = ["/usr/lib", "/usr/libexec"].map(|parent| format!("{parent}/{identifier}"));
    let mut cpio = Vec::new();
    let mut files = Vec::new();
    let mut total_size = 0u64;

    for path in &paths {
        let meta = path.symlink_metadata().map_err(|e| format!("{}: {e}", path.display()))?;
        let relative = path.strip_prefix(stage).map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        let installed = format!("/{relative}");
        if meta.is_dir() && !owned.iter().any(|own| installed == *own || installed.starts_with(&format!("{own}/"))) {
            continue;
        }
        let mtime = meta.mtime() as u32;
        let mode = meta.mode();
        let inode = files.len() as u32 + 1;

        let (body, digest, link_to) = if meta.is_dir() {
            (Vec::new(), String::new(), String::new())
        } else if meta.is_symlink() {
            let target = std::fs::read_link(path).map_err(|e| format!("{}: {e}", path.display()))?.to_string_lossy().into_owned();
            (target.clone().into_bytes(), String::new(), target)
        } else {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let digest = hex(&sha2::Sha256::digest(&bytes));
            total_size += bytes.len() as u64;
            (bytes, digest, String::new())
        };

        cpio_entry(&mut cpio, &format!(".{installed}"), mode, body.len() as u32, mtime, inode);
        cpio.extend_from_slice(&body);
        while cpio.len() % 4 != 0 {
            cpio.push(0);
        }

        files.push(File { installed, size: body.len() as u32, mode: mode as u16, mtime, digest, link_to });
    }

    cpio_entry(&mut cpio, "TRAILER!!!", 0, 0, 0, 0);
    Ok(Payload { cpio, files, total_size })
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        out.push(path.clone());
        if path.is_dir() && !path.is_symlink() {
            walk(&path, out);
        }
    }
}

pub fn write(stage: &Path, meta: &Metadata, dest: &Path) -> Result<(), String> {
    let payload = collect(stage, meta.identifier)?;

    let mut dirs: Vec<String> = Vec::new();
    let mut dir_indexes: Vec<u32> = Vec::new();
    let mut basenames: Vec<String> = Vec::new();

    for file in &payload.files {
        let (dir, base) = file.installed.rsplit_once('/').unwrap_or(("", &file.installed));
        let dir = format!("{dir}/");
        let index = match dirs.iter().position(|d| *d == dir) {
            Some(index) => index,
            None => {
                dirs.push(dir);
                dirs.len() - 1
            }
        };
        dir_indexes.push(index as u32);
        basenames.push(base.to_owned());
    }

    let count = payload.files.len();
    let epoch_prefix = if meta.epoch > 0 { format!("{}:", meta.epoch) } else { String::new() };
    let evr = format!("{epoch_prefix}{}-{}", meta.version, meta.release);

    let mut require_names = vec!["rpmlib(CompressedFileNames)".to_owned(), "rpmlib(FileDigests)".to_owned(), "rpmlib(PayloadFilesHavePrefix)".to_owned()];
    let mut require_versions = vec!["3.0.4-1".to_owned(), "4.6.0-1".to_owned(), "4.0-1".to_owned()];
    let mut require_flags = vec![RPMLIB_SENSE; 3];
    for dependency in &meta.requires {
        require_names.push(dependency.clone());
        require_versions.push(String::new());
        require_flags.push(0);
    }

    let entries = vec![
        (100, Value::StrArray(vec!["C".to_owned()])),
        (1000, Value::Str(meta.name.to_owned())),
        (1001, Value::Str(meta.version.to_owned())),
        (1002, Value::Str(meta.release.to_owned())),
        (1003, Value::Int32(vec![meta.epoch])),
        (1004, Value::I18n(meta.summary.to_owned())),
        (1005, Value::I18n(meta.description.to_owned())),
        (1006, Value::Int32(vec![std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs() as u32)])),
        (1009, Value::Int32(vec![payload.total_size as u32])),
        (1014, Value::Str(meta.license.to_owned())),
        (1011, Value::Str(meta.vendor.to_owned())),
        (1016, Value::I18n(meta.group.to_owned())),
        (1020, Value::Str(meta.url.to_owned())),
        (1021, Value::Str("linux".to_owned())),
        (1022, Value::Str(meta.arch.to_owned())),
        (1028, Value::Int32(payload.files.iter().map(|f| f.size).collect())),
        (1030, Value::Int16(payload.files.iter().map(|f| f.mode).collect())),
        (1033, Value::Int16(vec![0; count])),
        (1034, Value::Int32(payload.files.iter().map(|f| f.mtime).collect())),
        (1035, Value::StrArray(payload.files.iter().map(|f| f.digest.clone()).collect())),
        (1036, Value::StrArray(payload.files.iter().map(|f| f.link_to.clone()).collect())),
        (1037, Value::Int32(vec![0; count])),
        (1039, Value::StrArray(vec!["root".to_owned(); count])),
        (1040, Value::StrArray(vec!["root".to_owned(); count])),
        (1044, Value::Str(format!("{}-{}-{}.src.rpm", meta.name, meta.version, meta.release))),
        (1045, Value::Int32(vec![u32::MAX; count])),
        (1046, Value::Int32(vec![payload.cpio.len() as u32])),
        (1047, Value::StrArray(vec![meta.name.to_owned()])),
        (1048, Value::Int32(require_flags)),
        (1049, Value::StrArray(require_names)),
        (1050, Value::StrArray(require_versions)),
        (1094, Value::Int32(vec![SENSE_EQUAL])),
        (1095, Value::Int32(vec![1; count])),
        (1096, Value::Int32((1..=count as u32).collect())),
        (1097, Value::StrArray(vec![String::new(); count])),
        (1113, Value::StrArray(vec![evr.clone()])),
        (1116, Value::Int32(dir_indexes)),
        (1117, Value::StrArray(basenames)),
        (1118, Value::StrArray(dirs)),
        (1124, Value::Str("cpio".to_owned())),
        (1125, Value::Str("gzip".to_owned())),
        (1126, Value::Str("9".to_owned())),
        (5011, Value::Int32(vec![FILEDIGEST_SHA256])),
    ];

    let header_bytes = header(REGION_IMMUTABLE, entries);

    let compressed = {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &payload.cpio).and_then(|_| encoder.finish()).map_err(|e| format!("compressing the rpm payload: {e}"))?
    };

    let mut signed = header_bytes.clone();
    signed.extend_from_slice(&compressed);

    let signature = header(REGION_SIGNATURES, vec![
        (1000, Value::Int32(vec![signed.len() as u32])),
        (1004, Value::Bin(md5::Md5::digest(&signed).to_vec())),
        (1007, Value::Int32(vec![payload.cpio.len() as u32])),
        (273, Value::Str(hex(&sha2::Sha256::digest(&header_bytes)))),
    ]);

    let mut out = lead(meta);
    out.extend_from_slice(&signature);
    while !out.len().is_multiple_of(8) {
        out.push(0);
    }
    out.extend_from_slice(&signed);

    crate::util::write(dest, out)
}
