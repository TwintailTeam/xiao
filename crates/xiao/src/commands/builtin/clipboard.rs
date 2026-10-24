use crate::app::AppHandle;
use crate::commands::XiaoCommand;
use serde_json::Value;

pub struct Clipboard;

impl XiaoCommand for Clipboard {
    type Args = Value;
    type Output = Value;

    fn name() -> &'static str { "xiao:clipboard" }

    fn execute(app: &AppHandle, args: Self::Args) -> Result<Self::Output, String> {
        let action = args.get("action").and_then(|v| v.as_str()).ok_or("missing `action`")?;
        let clipboard = app.clipboard();
        let text = |key: &str| args.get(key).and_then(|v| v.as_str()).unwrap_or("");

        if action.starts_with("read") && !crate::config::page_may("readClipboard") {
            return Err(crate::config::page_denied("readClipboard"));
        }
        if (action.starts_with("write") || action == "clear") && !crate::config::page_may("writeClipboard") {
            return Err(crate::config::page_denied("writeClipboard"));
        }

        match action {
            "writeText" => clipboard.write_text(text("text"))?,
            "readText" => return Ok(Value::from(clipboard.read_text()?)),
            "writeHtml" => clipboard.write_html(text("html"), args.get("text").and_then(|v| v.as_str()))?,
            "readHtml" => return Ok(Value::from(clipboard.read_html()?)),
            "writeImage" => clipboard.write_image(text("image"))?,
            "readImage" => return Ok(Value::from(clipboard.read_image()?)),
            "clear" => clipboard.clear()?,
            "kinds" => return serde_json::to_value(clipboard.kinds()?).map_err(|e| e.to_string()),
            other => return Err(format!("unknown clipboard action `{other}`")),
        }

        Ok(Value::Null)
    }
}
