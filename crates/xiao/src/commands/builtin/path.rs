use crate::app::AppHandle;
use crate::commands::XiaoCommand;
use serde_json::Value;

pub struct Path;

impl XiaoCommand for Path {
    type Args = Value;
    type Output = Value;

    fn name() -> &'static str { "xiao:path" }

    fn execute(app: &AppHandle, args: Self::Args) -> Result<Self::Output, String> {
        let resolver = app.path();

        let Some(name) = args.get("name").and_then(|v| v.as_str()) else {
            let all: serde_json::Map<String, Value> = resolver
                .all()
                .into_iter()
                .map(|(key, dir)| (key.to_owned(), dir.map(|p| Value::from(p.display().to_string())).unwrap_or(Value::Null)))
                .collect();
            return Ok(Value::Object(all));
        };

        let dir = resolver.by_name(name).ok_or_else(|| format!("no path named `{name}` on this platform"))?;
        let dir = match args.get("join").and_then(|v| v.as_str()) {
            Some(relative) if !relative.is_empty() => {
                if !crate::config::is_relative_below(relative) {
                    return Err(format!("`join` must be a relative path inside `{name}`, not `{relative}`"));
                }
                dir.join(relative)
            }
            _ => dir,
        };

        if args.get("create").and_then(|v| v.as_bool()).unwrap_or(false) {
            if !crate::config::page_may("createDirectories") {
                return Err(crate::config::page_denied("createDirectories"));
            }
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        }

        Ok(Value::from(dir.display().to_string()))
    }
}
