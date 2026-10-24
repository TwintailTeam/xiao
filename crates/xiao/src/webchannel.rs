use crate::app::AppHandle;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Lower-level access to the page, from [`AppHandle::webchannel`].
pub struct WebChannelHandle<'a> {
    pub(crate) app: &'a AppHandle,
}

impl WebChannelHandle<'_> {
    /// Whether the current page has connected. Events emitted before it does are held back until then.
    pub fn is_attached(&self) -> bool { crate::app::is_attached() }

    /// Qt's `qwebchannel.js`, which the shell already loads into every page.
    pub fn script(&self) -> String { crate::read_resource(":/qtwebchannel/qwebchannel.js").unwrap_or_default() }

    /// Emits `event` with an already serialized JSON payload.
    pub fn emit_raw(&self, event: &str, payload_json: &str) { self.app.emit_json(event, payload_json.to_owned()); }

    /// Runs `javascript` in the page.
    pub fn eval(&self, javascript: &str) { self.send("eval", json!({ "script": javascript })); }

    /// Runs `javascript` in the page and returns an id; the result is emitted
    /// as JSON on `webchannel://result/<id>`.
    pub fn eval_result(&self, javascript: &str) -> String {
        let id = format!("w{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        self.send("eval", json!({ "script": javascript, "id": id }));
        id
    }

    /// Adds a script that runs in every page loaded from now on, before the
    /// page's own scripts. For the first page too, use [`Shell::user_script`](crate::Shell::user_script).
    pub fn inject(&self, name: &str, source: &str) { self.send("inject", json!({ "name": name, "source": source })); }

    /// Removes the scripts injected under `name`.
    pub fn remove_script(&self, name: &str) { self.send("removeScript", json!({ "name": name })); }

    fn send(&self, action: &str, args: serde_json::Value) { self.app.webchannel_command(action, args); }
}
