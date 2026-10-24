mod builtin;

use crate::app::AppHandle;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// A command the page can call with `invoke(name, args)`.
///
/// ```ignore
/// pub struct Greet;
///
/// #[derive(Deserialize)]
/// pub struct Args {
///     pub name: String,
/// }
///
/// impl XiaoCommand for Greet {
///     type Args = Args;
///     type Output = String;
///
///     fn name() -> &'static str { "greet" }
///
///     fn execute(_app: &AppHandle, args: Self::Args) -> Result<Self::Output, String> { Ok(format!("Hello, {}!", args.name)) }
/// }
/// ```
pub trait XiaoCommand {
    /// Deserialized from the page's arguments object.
    type Args: DeserializeOwned;
    /// Serialized as the value `invoke` resolves with.
    type Output: Serialize;

    /// Whether `execute` runs on its own thread instead of the Qt thread. Use
    /// it for slow work. Such a command must not touch Qt objects directly.
    const ASYNC: bool = false;

    /// The name the page invokes it by.
    fn name() -> &'static str;

    /// Runs the command. An `Err` rejects the page's `invoke` with the message.
    fn execute(app: &AppHandle, args: Self::Args) -> Result<Self::Output, String>;
}

pub(crate) type Handler = Arc<dyn Fn(&AppHandle, Value) -> Result<Value, String> + Send + Sync>;

/// A registered command. Created by [`commands!`](crate::commands).
#[derive(Clone)]
pub struct Command {
    pub(crate) handler: Handler,
    pub(crate) on_worker: bool,
}

/// Commands by name, as passed to [`Shell::commands`](crate::Shell::commands).
pub type Registry = HashMap<&'static str, Command>;

pub fn command_handler<C: XiaoCommand + 'static>() -> (&'static str, Command) {
    let handler: Handler = Arc::new(|app, args| {
        let parsed = serde_json::from_value(args).map_err(|e| format!("invalid arguments for `{}`: {e}", C::name()))?;
        let output = C::execute(app, parsed)?;
        serde_json::to_value(output).map_err(|e| e.to_string())
    });
    (C::name(), Command { handler, on_worker: C::ASYNC })
}

/// Builds a [`Registry`] from [`XiaoCommand`] types.
///
/// ```ignore
/// xiao::Shell::new(CONFIG_JSON, QML_ENTRY).commands(xiao::commands![Greet, Info]).run();
/// ```
#[macro_export]
macro_rules! commands {
    ($($command:ty),* $(,)?) => {{
        let mut registry = $crate::Registry::new();
        $({
            let (name, handler) = $crate::command_handler::<$command>();
            registry.insert(name, handler);
        })*
        registry
    }};
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

pub(crate) fn install(mut registry: Registry) {
    registry.extend(builtin::registry());
    let _ = REGISTRY.set(registry);
}

pub(crate) fn lookup(command: &str) -> Option<&'static Command> { REGISTRY.get().and_then(|r| r.get(command)) }
