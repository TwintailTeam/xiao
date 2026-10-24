use crate::app::AppHandle;
use crate::commands::XiaoCommand;
use serde_json::Value;

pub struct Window;

impl XiaoCommand for Window {
    type Args = Value;
    type Output = Value;

    fn name() -> &'static str { "xiao:window" }

    fn execute(app: &AppHandle, args: Self::Args) -> Result<Self::Output, String> {
        let action = args.get("action").and_then(|v| v.as_str()).ok_or("missing `action`")?;
        let window = app.window();
        let flag = || args.get("value").and_then(|v| v.as_bool()).unwrap_or(false);
        let number = |key: &str| args.get(key).and_then(|v| v.as_i64()).unwrap_or(0);

        match action {
            "show" => window.show(),
            "hide" => window.hide(),
            "close" => window.close(),
            "destroy" => window.destroy(),
            "minimize" => window.minimize(),
            "maximize" => window.maximize(),
            "unmaximize" => window.unmaximize(),
            "toggleMaximize" => window.toggle_maximize(),
            "center" => window.center(),
            "focus" => window.focus(),
            "setFullscreen" => window.set_fullscreen(flag()),
            "setResizable" => window.set_resizable(flag()),
            "setAlwaysOnTop" => window.set_always_on_top(flag()),
            "setPreventClose" => window.set_prevent_close(flag()),
            "setTitle" => window.set_title(args.get("value").and_then(|v| v.as_str()).unwrap_or("")),
            "setSize" => window.set_size(number("width") as u32, number("height") as u32),
            "setMinSize" => window.set_min_size(number("width") as u32, number("height") as u32),
            "setPosition" => window.set_position(number("x") as i32, number("y") as i32),
            other => return Err(format!("unknown window action `{other}`")),
        }

        Ok(Value::Null)
    }
}
