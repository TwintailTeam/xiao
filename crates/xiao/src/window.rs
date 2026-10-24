use crate::app::AppHandle;
use serde::Serialize;
use serde_json::json;
use std::sync::{OnceLock, RwLock};

/// The window's visibility, as Qt reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Visibility {
    Hidden,
    AutomaticVisibility,
    Windowed,
    Minimized,
    Maximized,
    FullScreen,
}

impl Visibility {
    fn from_i64(value: i64) -> Option<Self> {
        Some(match value {
            0 => Self::Hidden,
            1 => Self::AutomaticVisibility,
            2 => Self::Windowed,
            3 => Self::Minimized,
            4 => Self::Maximized,
            5 => Self::FullScreen,
            _ => return None,
        })
    }
}

/// The desktop's color scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    Unknown,
    Light,
    Dark,
}

impl Theme {
    fn from_name(name: &str) -> Self {
        match name {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::Unknown,
        }
    }
}

/// The window's last reported state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct WindowState {
    pub visibility: Visibility,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub focused: bool,
    pub scale_factor: f64,
    pub theme: Theme,
}

impl WindowState {
    fn from_config() -> Self {
        let win = &crate::config::config()["app"]["windows"][0];
        let number = |key: &str, fallback: i64| win[key].as_i64().unwrap_or(fallback);

        let visibility = if win["fullscreen"].as_bool().unwrap_or(false) {
            Visibility::FullScreen
        } else if win["maximized"].as_bool().unwrap_or(false) {
            Visibility::Maximized
        } else if win["visible"].as_bool().unwrap_or(true) {
            Visibility::Windowed
        } else {
            Visibility::Hidden
        };

        Self {
            visibility,
            width: number("width", 800) as u32,
            height: number("height", 600) as u32,
            x: number("x", 0) as i32,
            y: number("y", 0) as i32,
            focused: false,
            scale_factor: 1.0,
            theme: Theme::Unknown,
        }
    }
}

fn state() -> &'static RwLock<WindowState> {
    static STATE: OnceLock<RwLock<WindowState>> = OnceLock::new();
    STATE.get_or_init(|| RwLock::new(WindowState::from_config()))
}

pub(crate) fn record(event: &str, payload: &serde_json::Value) {
    let Some(name) = event.strip_prefix("window://") else { return };

    let mut state = state().write().unwrap();
    match name {
        "visibility" => {
            if let Some(value) = payload["visibility"].as_i64().and_then(Visibility::from_i64) {
                state.visibility = value;
            }
        }
        "resized" => {
            if let Some(width) = payload["width"].as_u64() {
                state.width = width as u32;
            }
            if let Some(height) = payload["height"].as_u64() {
                state.height = height as u32;
            }
        }
        "moved" => {
            if let Some(x) = payload["x"].as_i64() {
                state.x = x as i32;
            }
            if let Some(y) = payload["y"].as_i64() {
                state.y = y as i32;
            }
        }
        "focus" => state.focused = true,
        "blur" => state.focused = false,
        "scale-changed" => {
            if let Some(scale) = payload["scaleFactor"].as_f64() {
                state.scale_factor = scale;
            }
        }
        "theme-changed" => {
            if let Some(theme) = payload["theme"].as_str() {
                state.theme = Theme::from_name(theme);
            }
        }
        _ => {}
    }
}

/// The app's window, from [`AppHandle::window`].
///
/// Commands are queued to the Qt thread and return immediately.
pub struct WindowHandle<'a> {
    pub(crate) app: &'a AppHandle,
}

impl WindowHandle<'_> {
    fn send(&self, action: &str, args: serde_json::Value) { self.app.window_command(action, args); }

    pub fn show(&self) { self.send("show", json!({})); }

    pub fn hide(&self) { self.send("hide", json!({})); }

    /// Asks the window to close, as its close button does. See [`set_prevent_close`](Self::set_prevent_close).
    pub fn close(&self) { self.send("close", json!({})); }

    /// Closes the window and quits the app.
    pub fn destroy(&self) { self.send("destroy", json!({})); }

    pub fn minimize(&self) { self.send("minimize", json!({})); }

    pub fn maximize(&self) { self.send("maximize", json!({})); }

    pub fn unmaximize(&self) { self.send("unmaximize", json!({})); }

    pub fn toggle_maximize(&self) { self.send("toggleMaximize", json!({})); }

    pub fn set_fullscreen(&self, fullscreen: bool) { self.send("setFullscreen", json!({ "value": fullscreen })); }

    pub fn center(&self) { self.send("center", json!({})); }

    pub fn focus(&self) { self.send("focus", json!({})); }

    pub fn set_title(&self, title: &str) { self.send("setTitle", json!({ "value": title })); }

    pub fn set_size(&self, width: u32, height: u32) { self.send("setSize", json!({ "width": width, "height": height })); }

    pub fn set_min_size(&self, width: u32, height: u32) { self.send("setMinSize", json!({ "width": width, "height": height })); }

    pub fn set_position(&self, x: i32, y: i32) { self.send("setPosition", json!({ "x": x, "y": y })); }

    pub fn set_resizable(&self, resizable: bool) { self.send("setResizable", json!({ "value": resizable })); }

    pub fn set_always_on_top(&self, on_top: bool) { self.send("setAlwaysOnTop", json!({ "value": on_top })); }

    /// When `true`, closing the window only emits `window://close-requested`.
    pub fn set_prevent_close(&self, prevent: bool) { self.send("setPreventClose", json!({ "value": prevent })); }

    /// Sets the window icon from encoded image bytes in any format Qt reads.
    pub fn set_icon(&self, image: &[u8]) {
        let image = image.to_vec();
        self.app.run_on_main_thread(move || {
            if !crate::set_window_icon_data(&image) {
                eprintln!("[window] set_icon: Qt cannot decode the image");
            }
        });
    }

    pub fn state(&self) -> WindowState { *state().read().unwrap() }

    pub fn visibility(&self) -> Visibility { self.state().visibility }

    pub fn is_minimized(&self) -> bool { self.visibility() == Visibility::Minimized }

    pub fn is_maximized(&self) -> bool { self.visibility() == Visibility::Maximized }

    pub fn is_fullscreen(&self) -> bool { self.visibility() == Visibility::FullScreen }

    pub fn is_visible(&self) -> bool { self.visibility() != Visibility::Hidden }

    pub fn is_focused(&self) -> bool { self.state().focused }

    /// The device pixel ratio of the window's screen.
    pub fn scale_factor(&self) -> f64 { self.state().scale_factor }

    pub fn theme(&self) -> Theme { self.state().theme }

    pub fn size(&self) -> (u32, u32) {
        let state = self.state();
        (state.width, state.height)
    }

    pub fn position(&self) -> (i32, i32) {
        let state = self.state();
        (state.x, state.y)
    }
}
