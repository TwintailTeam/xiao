use crate::app::AppHandle;
use crate::commands::XiaoCommand;
use serde_json::Value;

pub struct Tray;

impl XiaoCommand for Tray {
    type Args = Value;
    type Output = Value;

    fn name() -> &'static str { "xiao:tray" }

    fn execute(app: &AppHandle, args: Self::Args) -> Result<Self::Output, String> {
        let action = args.get("action").and_then(|v| v.as_str()).ok_or("missing `action`")?;
        let tray = app.tray();
        let text = |key: &str| args.get(key).and_then(|v| v.as_str()).unwrap_or("");

        match action {
            "show" => tray.show(),
            "hide" => tray.hide(),
            "setTooltip" => tray.set_tooltip(text("value")),
            "setIcon" => tray.set_icon(text("value")),
            "notify" => tray.notify(text("title"), text("message")),
            "setMenuOnLeftClick" => tray.set_menu_on_left_click(args.get("value").and_then(|v| v.as_bool()).unwrap_or(true)),
            "setMenu" => {
                let items: Vec<crate::tray::MenuItem> =
                    serde_json::from_value(args.get("items").cloned().unwrap_or(Value::Null)).map_err(|e| format!("bad menu items: {e}"))?;
                tray.set_menu(&items);
            }
            other => return Err(format!("unknown tray action `{other}`")),
        }

        Ok(Value::Null)
    }
}
