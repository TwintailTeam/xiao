use crate::app::AppHandle;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Native dialogs, from [`AppHandle::dialog`].
///
/// Each method returns a request id at once; the answer is emitted as
/// `dialog://result/<id>`. Requests made while a dialog is open wait their turn.
pub struct DialogHandle<'a> {
    pub(crate) app: &'a AppHandle,
}

impl DialogHandle<'_> {
    fn request(&self, kind: &str, mut args: Value) -> String {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("d{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)));
        if let Some(obj) = args.as_object_mut() {
            obj.insert("id".into(), Value::from(id.clone()));
            obj.insert("kind".into(), Value::from(kind));
        }
        self.app.dialog_command(args);
        id
    }

    /// Picks one or more files. `args`: `{ title, filters, defaultPath }`.
    pub fn open(&self, args: Value) -> String { self.request("open", args) }

    /// Picks a folder. `args`: `{ title, defaultPath }`.
    pub fn folder(&self, args: Value) -> String { self.request("folder", args) }

    /// Picks a file to save to. `args`: `{ title, filters, defaultPath }`.
    pub fn save(&self, args: Value) -> String { self.request("save", args) }

    /// Shows a message box. `args`: `{ title, text, detail, buttons }`, where
    /// `buttons` is `"ok"`, `"okcancel"` or `"yesno"`.
    pub fn message(&self, args: Value) -> String { self.request("message", args) }
}
