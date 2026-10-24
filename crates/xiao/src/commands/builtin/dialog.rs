use crate::app::AppHandle;
use crate::commands::XiaoCommand;
use serde_json::Value;

pub struct Dialog;

impl XiaoCommand for Dialog {
    type Args = Value;
    type Output = Value;

    fn name() -> &'static str { "xiao:dialog" }

    fn execute(app: &AppHandle, args: Self::Args) -> Result<Self::Output, String> {
        let kind = args.get("kind").and_then(|v| v.as_str()).ok_or("missing `kind`")?;
        let dialog = app.dialog();

        if matches!(kind, "open" | "folder" | "save") && !crate::config::page_may("fileDialogs") {
            return Err(crate::config::page_denied("fileDialogs"));
        }

        let id = match kind {
            "open" => dialog.open(args.clone()),
            "folder" => dialog.folder(args.clone()),
            "save" => dialog.save(args.clone()),
            "message" => dialog.message(args.clone()),
            other => return Err(format!("unknown dialog kind `{other}`")),
        };

        Ok(Value::from(id))
    }
}
