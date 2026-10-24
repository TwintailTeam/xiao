use sha2::Digest as _;

const RUNTIMES: &[(&str, &[u8], &str)] = &[
    ("x86_64", include_bytes!("../runtime/type2-runtime-20251108-x86_64"), "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"),
    ("aarch64", include_bytes!("../runtime/type2-runtime-20251108-aarch64"), "00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444"),
];

pub fn appimage_runtime(arch: &str) -> Result<&'static [u8], String> {
    let (_, runtime, sha256) = RUNTIMES.iter().find(|(name, ..)| *name == arch).ok_or_else(|| format!("no pinned AppImage runtime for {arch}"))?;
    if sha256_hex(runtime) != *sha256 {
        return Err(format!("the {arch} AppImage runtime built into xiao does not match its pinned checksum"));
    }
    Ok(runtime)
}

fn sha256_hex(bytes: &[u8]) -> String { sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect() }
