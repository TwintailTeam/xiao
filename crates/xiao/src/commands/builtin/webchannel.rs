use crate::app::AppHandle;
use crate::commands::XiaoCommand;
use serde_json::{Value, json};

pub struct WebChannel;

impl XiaoCommand for WebChannel {
    type Args = Value;
    type Output = Value;

    fn name() -> &'static str { "xiao:webchannel" }

    fn execute(app: &AppHandle, args: Self::Args) -> Result<Self::Output, String> {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("info") {
            "info" => Ok(json!({
                "attached": app.webchannel().is_attached(),
                "csp": crate::config::active_csp(),
                "storage": crate::config::web_storage(),
            })),
            other => Err(format!("unknown webchannel action `{other}`")),
        }
    }
}
