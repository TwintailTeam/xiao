use serde_json::Value;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "xiao.conf.json";
const CONFIG_ENV: &str = "XIAO_CONFIG";

pub struct Conf {
    pub root: PathBuf,
    pub app: PathBuf,
    pub json: Value,
    pub files: Vec<PathBuf>,
    extra: Vec<String>,
}

impl Conf {
    pub fn load(target_os: &str, extra: &[String]) -> Result<Self, String> {
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let root = cwd
            .ancestors()
            .find(|dir| dir.join(CONFIG_FILE).is_file())
            .ok_or_else(|| format!("cannot find {CONFIG_FILE} in {} or any parent directory", cwd.display()))?
            .to_path_buf();

        let mut files = vec![root.join(CONFIG_FILE)];
        let mut json = read_json(&files[0])?;

        let platform = root.join(format!("xiao.{target_os}.conf.json"));
        if platform.is_file() {
            merge(&mut json, read_json(&platform)?);
            files.push(platform);
        }
        for source in extra {
            merge(&mut json, patch(source)?);
            if !is_inline(source) {
                files.push(cwd.join(source));
            }
        }

        let app = find_app_package(&root, &json)?;
        Ok(Self { root, app, json, files, extra: extra.to_vec() })
    }

    pub fn cargo(&self) -> Result<std::process::Command, String> {
        let mut cargo = std::process::Command::new("cargo");
        cargo.current_dir(&self.app).envs(crate::qt::cargo_env()?);
        if self.extra.is_empty() {
            cargo.env_remove(CONFIG_ENV);
        } else {
            let patches = self.extra.iter().map(|source| patch(source)).collect::<Result<Vec<_>, _>>()?;
            cargo.env(CONFIG_ENV, Value::Array(patches).to_string());
        }
        Ok(cargo)
    }

    pub fn target_dir(&self) -> PathBuf { self.app.join("target") }

    pub fn str_at(&self, path: &[&str]) -> Option<&str> { path.iter().fold(&self.json, |node, key| &node[*key]).as_str().filter(|s| !s.is_empty()) }

    pub fn strings_at(&self, path: &[&str]) -> Vec<String> {
        let node = path.iter().fold(&self.json, |node, key| &node[*key]);
        node.as_array().into_iter().flatten().filter_map(|item| item.as_str().map(str::to_owned)).collect()
    }

    pub fn product_name(&self) -> &str { self.str_at(&["productName"]).unwrap_or("app") }

    pub fn binary_name(&self) -> &str { self.str_at(&["mainBinaryName"]).unwrap_or_else(|| self.product_name()) }

    #[cfg(target_os = "linux")]
    pub fn package_name(&self) -> String {
        let name: String =
            self.product_name().chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '+' | '.') { c.to_ascii_lowercase() } else { '-' }).collect();
        let name = name.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if name.len() < 2 { self.binary_name().to_ascii_lowercase() } else { name.to_owned() }
    }

    pub fn version(&self) -> &str { self.str_at(&["version"]).unwrap_or("0.0.0") }

    pub fn identifier(&self) -> &str { self.str_at(&["identifier"]).unwrap_or("com.example.app") }

    pub fn deep_link_schemes(&self) -> Vec<String> {
        self.strings_at(&["app", "deepLink", "schemes"])
            .into_iter()
            .map(|scheme| scheme.trim().trim_end_matches("://").trim_end_matches(':').to_ascii_lowercase())
            .filter(|scheme| !scheme.is_empty())
            .collect()
    }

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
                let path = self.root.join(p);
                let stem = Path::new(p).file_stem()?.to_string_lossy().into_owned();
                let size = match stem.split_once('x') {
                    Some((w, h)) if w == h => w.parse().ok()?,
                    _ => 512,
                };
                path.is_file().then_some((size, path))
            })
            .collect()
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))
}

fn is_inline(source: &str) -> bool { source.trim_start().starts_with('{') }

fn patch(source: &str) -> Result<Value, String> {
    if is_inline(source) { serde_json::from_str(source).map_err(|e| format!("--config {source} is not valid JSON: {e}")) } else { read_json(Path::new(source)) }
}

fn merge(base: &mut Value, patch: Value) {
    let Value::Object(entries) = patch else {
        *base = patch;
        return;
    };
    if !base.is_object() {
        *base = Value::Object(serde_json::Map::new());
    }
    let Value::Object(target) = base else { return };
    for (key, value) in entries {
        if value.is_null() {
            target.remove(&key);
        } else {
            merge(target.entry(key).or_insert(Value::Null), value);
        }
    }
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
