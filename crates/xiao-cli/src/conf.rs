use serde_json::Value;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "xiao.conf.json";

pub struct Conf {
    pub root: PathBuf,
    pub app: PathBuf,
    pub json: Value,
}

impl Conf {
    pub fn load() -> Result<Self, String> {
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let root = find_root(&cwd).ok_or_else(|| format!("cannot find {} in {} or any parent directory", CONFIG_FILE, cwd.display()))?;

        let path = root.join(CONFIG_FILE);
        let raw = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let json: Value = serde_json::from_str(&raw).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?;

        let app = find_app_package(&root, &json)?;
        Ok(Self { root, app, json })
    }

    pub fn target_dir(&self) -> PathBuf { self.app.join("target") }

    pub fn str_at(&self, path: &[&str]) -> Option<&str> {
        let mut node = &self.json;
        for key in path {
            node = node.get(key)?;
        }
        node.as_str().filter(|s| !s.is_empty())
    }

    pub fn strings_at(&self, path: &[&str]) -> Vec<String> {
        let mut node = &self.json;
        for key in path {
            match node.get(key) {
                Some(next) => node = next,
                None => return Vec::new(),
            }
        }
        node.as_array().map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
    }

    pub fn product_name(&self) -> &str { self.str_at(&["productName"]).unwrap_or("app") }

    pub fn binary_name(&self) -> &str { self.str_at(&["mainBinaryName"]).unwrap_or_else(|| self.product_name()) }

    pub fn version(&self) -> &str { self.str_at(&["version"]).unwrap_or("0.0.0") }

    pub fn identifier(&self) -> &str { self.str_at(&["identifier"]).unwrap_or("com.example.app") }

    pub fn dev_url(&self) -> String {
        std::env::var("XIAO_DEV_URL").unwrap_or_else(|_| self.str_at(&["build", "devUrl"]).unwrap_or("http://localhost:1420").to_owned())
    }

    pub fn deep_link_schemes(&self) -> Vec<String> {
        self.strings_at(&["app", "deepLink", "schemes"])
            .into_iter()
            .map(|scheme| scheme.trim().trim_end_matches("://").trim_end_matches(':').to_ascii_lowercase())
            .filter(|scheme| !scheme.is_empty())
            .collect()
    }

    pub fn asset(&self, relative: &str) -> PathBuf { self.root.join(relative) }

    pub fn icons(&self) -> Vec<String> {
        match &self.json["bundle"]["icon"] {
            Value::Array(_) => self.strings_at(&["bundle", "icon"]),
            Value::String(one) => vec![one.clone()],
            _ => Vec::new(),
        }
    }

    pub fn icon_pngs(&self) -> Vec<(u32, PathBuf)> {
        self.icons()
            .iter()
            .filter(|p| p.ends_with(".png"))
            .filter_map(|p| {
                let path = self.asset(p);
                let stem = Path::new(p).file_stem()?.to_string_lossy().into_owned();
                let size = match stem.split_once('x') {
                    Some((w, h)) if w == h => w.parse().ok()?,
                    _ => 512,
                };
                path.is_file().then_some((size, path))
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    pub fn largest_png(&self) -> Option<PathBuf> { self.icon_pngs().into_iter().max_by_key(|(size, _)| *size).map(|(_, path)| path) }
}

fn find_app_package(root: &Path, json: &Value) -> Result<PathBuf, String> {
    if let Some(named) = json["build"]["appDir"].as_str().filter(|s| !s.is_empty()) {
        let dir = root.join(named);
        return if dir.join("Cargo.toml").is_file() { Ok(dir) } else { Err(format!("build.appDir is {named}, but there is no Cargo.toml in it")) };
    }

    let mut found: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|e| format!("{}: {e}", root.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| std::fs::read_to_string(path.join("Cargo.toml")).is_ok_and(|manifest| manifest.contains("[package]")))
        .collect();
    found.sort();

    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(format!("no cargo package next to {}: the app's Rust needs its own directory with a Cargo.toml", CONFIG_FILE)),
        _ => Err(format!(
            "several cargo packages next to {} ({}); name the app's one in build.appDir",
            CONFIG_FILE,
            found.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn find_root(from: &Path) -> Option<PathBuf> {
    let mut dir = Some(from);
    while let Some(current) = dir {
        if current.join(CONFIG_FILE).is_file() {
            return Some(current.to_path_buf());
        }
        dir = current.parent();
    }
    None
}
