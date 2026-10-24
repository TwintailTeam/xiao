use crate::app::{AppHandle, EventId};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// What activated the tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TrayActivation {
    Unknown,
    /// The context menu was requested, usually a right click.
    Context,
    DoubleClick,
    /// A left click.
    Trigger,
    MiddleClick,
}

impl TrayActivation {
    fn from_i64(value: i64) -> Self {
        match value {
            1 => Self::Context,
            2 => Self::DoubleClick,
            3 => Self::Trigger,
            4 => Self::MiddleClick,
            _ => Self::Unknown,
        }
    }
}

/// An entry in the tray menu. Clicking it emits `tray://menu` with its `id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuItem {
    pub id: String,
    pub text: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub separator: bool,
    /// `Some` makes the entry checkable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
}

fn default_true() -> bool { true }

impl MenuItem {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self { id: id.into(), text: text.into(), enabled: true, separator: false, checked: None }
    }

    pub fn separator() -> Self { Self { id: String::new(), text: String::new(), enabled: true, separator: true, checked: None } }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Makes the entry checkable, initially `checked`.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }
}

/// The tray icon, from [`AppHandle::tray`].
pub struct TrayHandle<'a> {
    pub(crate) app: &'a AppHandle,
}

impl TrayHandle<'_> {
    fn send(&self, action: &str, args: serde_json::Value) { self.app.tray_command(action, args); }

    pub fn show(&self) { self.send("show", json!({})); }

    pub fn hide(&self) { self.send("hide", json!({})); }

    pub fn set_tooltip(&self, tooltip: &str) { self.send("setTooltip", json!({ "value": tooltip })); }

    /// Sets the icon from a URL Qt can load, such as `qrc:` or `file:`.
    pub fn set_icon(&self, icon: &str) { self.send("setIcon", json!({ "value": icon })); }

    pub fn set_menu(&self, items: &[MenuItem]) { self.send("setMenu", json!({ "items": items })); }

    /// Whether a left click opens the menu. On by default. Wayland status bars
    /// decide this themselves.
    pub fn set_menu_on_left_click(&self, value: bool) { self.send("setMenuOnLeftClick", json!({ "value": value })); }

    /// Shows a notification, through the desktop's notification service where there is one.
    pub fn notify(&self, title: &str, message: &str) { self.send("showMessage", json!({ "title": title, "message": message })); }

    pub fn set_item_text(&self, id: &str, text: &str) { self.send("updateItem", json!({ "id": id, "text": text })); }

    pub fn set_item_enabled(&self, id: &str, enabled: bool) { self.send("updateItem", json!({ "id": id, "enabled": enabled })); }

    /// Sets the check state of an entry created with [`MenuItem::checked`].
    pub fn set_item_checked(&self, id: &str, checked: bool) { self.send("updateItem", json!({ "id": id, "checked": checked })); }

    /// Calls `handler` with the `id` of each menu entry that is clicked.
    pub fn on_menu_click<F>(&self, handler: F) -> EventId
    where F: Fn(&AppHandle, &str) + Send + Sync + 'static {
        self.app.listen("tray://menu", move |app, payload| {
            handler(app, payload["id"].as_str().unwrap_or_default());
        })
    }

    pub fn on_activated<F>(&self, handler: F) -> EventId
    where F: Fn(&AppHandle, TrayActivation) + Send + Sync + 'static {
        self.app.listen("tray://activated", move |app, payload| {
            handler(app, TrayActivation::from_i64(payload["reason"].as_i64().unwrap_or(0)));
        })
    }

    /// Calls `handler` when a notification from [`notify`](Self::notify) is clicked.
    pub fn on_message_clicked<F>(&self, handler: F) -> EventId
    where F: Fn(&AppHandle) + Send + Sync + 'static {
        self.app.listen("tray://message-clicked", move |app, _| handler(app))
    }
}
