use crate::app::AppHandle;
use crate::commands::XiaoCommand;
use serde_json::Value;

pub struct Log;

impl XiaoCommand for Log {
    type Args = Value;
    type Output = Value;

    fn name() -> &'static str { "xiao:log" }

    fn execute(_app: &AppHandle, args: Self::Args) -> Result<Self::Output, String> {
        let level = match args.get("level").and_then(|v| v.as_str()).unwrap_or("info") {
            "trace" => log::Level::Trace,
            "debug" => log::Level::Debug,
            "info" => log::Level::Info,
            "warn" => log::Level::Warn,
            "error" => log::Level::Error,
            other => return Err(format!("unknown log level `{other}`")),
        };
        let message = args.get("message").and_then(|v| v.as_str()).unwrap_or_default();
        log::log!(target: "webview", level, "{message}");
        Ok(Value::Null)
    }
}
