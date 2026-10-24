use crate::app::AppHandle;
use crate::commands::XiaoCommand;
use serde_json::Value;

pub struct DeepLink;

impl XiaoCommand for DeepLink {
    type Args = Value;
    type Output = Value;

    fn name() -> &'static str { "xiao:deep-link" }

    fn execute(app: &AppHandle, args: Self::Args) -> Result<Self::Output, String> {
        let action = args.get("action").and_then(|v| v.as_str()).ok_or("missing `action`")?;
        let deep_link = app.deep_link();
        let scheme = || args.get("scheme").and_then(|v| v.as_str()).filter(|scheme| !scheme.is_empty()).ok_or("missing `scheme`");

        if matches!(action, "register" | "unregister") && !crate::config::page_may("registerDeepLinks") {
            return Err(crate::config::page_denied("registerDeepLinks"));
        }

        match action {
            "current" => Ok(Value::from(deep_link.current())),
            "register" => deep_link.register(scheme()?).map(|_| Value::Null),
            "unregister" => deep_link.unregister(scheme()?).map(|_| Value::Null),
            "isRegistered" => Ok(Value::from(deep_link.is_registered(scheme()?))),
            other => Err(format!("unknown deep-link action `{other}`")),
        }
    }
}
