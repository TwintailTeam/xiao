use crate::util::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

const CHANNEL: &str = "https://aka.ms/vs/17/release/channel";

pub fn crt_dir(arch: &str) -> Result<PathBuf, String> {
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "i686" | "i586" | "x86" => "x86",
        other => return Err(format!("there is no MSVC runtime for {other}")),
    };

    if let Some(redist) = std::env::var_os("VCToolsRedistDir") {
        let dir = PathBuf::from(redist).join(arch);
        return newest_crt(&dir).ok_or_else(|| format!("VCToolsRedistDir has no {}/Microsoft.VC*.CRT", dir.display()));
    }

    let cache = cache_root()?.join("xiao/vcredist").join(arch);
    if !cache.join(".version").is_file() {
        download(arch, &cache)?;
    }
    Ok(cache)
}

fn newest_crt(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            path.is_dir() && name.starts_with("Microsoft.VC") && name.ends_with(".CRT")
        })
        .collect();
    found.sort();
    found.pop()
}

fn download(arch: &str, dest: &Path) -> Result<(), String> {
    let channel = json(CHANNEL)?;
    let manifest_url = channel["channelItems"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| item["type"] == "Manifest")
        .and_then(|item| item["payloads"][0]["url"].as_str())
        .ok_or("the Visual Studio channel lists no manifest")?;
    let manifest = json(manifest_url)?;

    let suffix = format!(".crt.redist.{arch}.base");
    let package = manifest["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|package| package["id"].as_str().map(str::to_ascii_lowercase).is_some_and(|id| id.starts_with("microsoft.vc.") && id.ends_with(&suffix)))
        .max_by_key(|package| version(package["version"].as_str().unwrap_or_default()))
        .ok_or_else(|| format!("the Visual Studio manifest has no {arch} CRT redist package"))?;
    let release = package["version"].as_str().unwrap_or_default();
    let payload = &package["payloads"][0];
    let (Some(url), Some(sha256)) = (payload["url"].as_str(), payload["sha256"].as_str()) else {
        return Err(format!("the {arch} CRT redist package has no download"));
    };

    log("bundle", CYAN, &format!("windows-portable: downloading MSVC runtime {release} ({arch})"));
    let vsix = fetch(url)?;
    let digest: String = Sha256::digest(&vsix).iter().map(|byte| format!("{byte:02x}")).collect();
    if !digest.eq_ignore_ascii_case(sha256) {
        return Err(format!("the CRT redist package from {url} does not match its sha256"));
    }

    let dlls = unzip(&vsix, |name| is_release_crt(name, arch)).ok_or("the CRT redist package is not a readable zip")?;
    if dlls.is_empty() {
        return Err(format!("the CRT redist package has no {arch}/Microsoft.VC*.CRT DLLs"));
    }
    reset_dir(dest)?;
    for (name, contents) in &dlls {
        let file = dest.join(name.rsplit('/').next().unwrap_or(name));
        std::fs::write(&file, contents).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    write(&dest.join(".version"), release)
}

fn is_release_crt(name: &str, arch: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    let [.., arch_dir, crt_dir, file] = parts.as_slice() else { return false };
    !name.contains("debug_nonredist")
        && arch_dir.eq_ignore_ascii_case(arch)
        && crt_dir.starts_with("Microsoft.VC")
        && crt_dir.ends_with(".CRT")
        && file.to_ascii_lowercase().ends_with(".dll")
}

fn version(text: &str) -> Vec<u32> { text.split('.').filter_map(|part| part.parse().ok()).collect() }

fn json(url: &str) -> Result<Value, String> { serde_json::from_slice(&fetch(url)?).map_err(|e| format!("{url}: {e}")) }

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let output = std::process::Command::new("curl")
        .args(["-fsSL", "--retry", "2", url])
        .output()
        .map_err(|e| format!("curl: {e} (needed to download the MSVC runtime, or set VCToolsRedistDir)"))?;
    if !output.status.success() {
        return Err(format!("curl {url} failed ({}): {}", output.status, String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(output.stdout)
}

fn cache_root() -> Result<PathBuf, String> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from).ok_or_else(|| format!("{name} is not set"));
    if cfg!(windows) {
        var("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Caches"))
    } else {
        var("XDG_CACHE_HOME").or_else(|_| var("HOME").map(|home| home.join(".cache")))
    }
}

fn unzip(bytes: &[u8], wanted: impl Fn(&str) -> bool) -> Option<Vec<(String, Vec<u8>)>> {
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])));
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);

    let end = (0..bytes.len().saturating_sub(21)).rev().find(|&at| u32_at(at) == Some(0x0605_4b50))?;
    let count = u16_at(end + 10)?;
    let mut at = u32_at(end + 16)?;
    let mut found = Vec::new();

    for _ in 0..count {
        if u32_at(at)? != 0x0201_4b50 {
            return None;
        }
        let method = u16_at(at + 10)?;
        let size = u32_at(at + 20)?;
        let name_len = u16_at(at + 28)?;
        let local = u32_at(at + 42)?;
        let name = String::from_utf8_lossy(bytes.get(at + 46..at + 46 + name_len)?).into_owned();
        at += 46 + name_len + u16_at(at + 30)? + u16_at(at + 32)?;
        if !wanted(&name) {
            continue;
        }

        let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let data = bytes.get(start..start + size)?;
        let contents = match method {
            0 => data.to_vec(),
            8 => {
                let mut inflated = Vec::new();
                flate2::read::DeflateDecoder::new(data).read_to_end(&mut inflated).ok()?;
                inflated
            }
            _ => return None,
        };
        found.push((name, contents));
    }
    Some(found)
}
