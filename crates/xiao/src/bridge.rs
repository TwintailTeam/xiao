use crate::app::{self, AppHandle};
use crate::commands;
use crate::config;
use crate::qstring::QString;
use core::pin::Pin;
use cxx_qt::{CxxQtThread, Threading};
use serde_json::{Value, json};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("xiao/cpp/qstring.h");
        type QString = crate::qstring::QString;
    }

    extern "RustQt" {
        #[qsignal]
        #[cxx_name = "windowCommand"]
        fn window_command(self: Pin<&mut Backend>, action: QString, args: QString);
        #[qsignal]
        #[cxx_name = "trayCommand"]
        fn tray_command(self: Pin<&mut Backend>, action: QString, args: QString);
        #[qsignal]
        #[cxx_name = "dialogCommand"]
        fn dialog_command(self: Pin<&mut Backend>, args: QString);
        #[qsignal]
        #[cxx_name = "webChannelCommand"]
        fn web_channel_command(self: Pin<&mut Backend>, action: QString, args: QString);
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type Backend = super::BackendRust;
        #[qinvokable]
        #[cxx_name = "startUrl"]
        fn start_url(&self) -> QString;
        #[qinvokable]
        #[cxx_name = "isDev"]
        fn is_dev(&self) -> bool;
        #[qinvokable]
        fn config(&self) -> QString;
        #[qinvokable]
        #[cxx_name = "cspScript"]
        fn csp_script(&self) -> QString;
        #[qinvokable]
        #[cxx_name = "userScripts"]
        fn user_scripts(&self) -> QString;
        #[qinvokable]
        #[cxx_name = "webStorage"]
        fn web_storage(&self) -> QString;
        #[qinvokable]
        #[cxx_name = "trayIconUrl"]
        fn tray_icon_url(&self) -> QString;
        #[qinvokable]
        fn notify(&self, title: &QString, body: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "injectsScripts"]
        fn injects_scripts(&self) -> bool;
        #[qinvokable]
        #[cxx_name = "isAppUrl"]
        fn is_app_url(&self, url: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "openExternal"]
        fn open_external(&self, url: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "pageLeaving"]
        fn page_leaving(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "restartApp"]
        fn restart_app(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "notifyWindowEvent"]
        fn notify_window_event(self: Pin<&mut Self>, name: &QString, payload: &QString);
        #[qinvokable]
        #[cxx_name = "notifyTrayEvent"]
        fn notify_tray_event(self: Pin<&mut Self>, name: &QString, payload: &QString);
        #[qinvokable]
        #[cxx_name = "notifyDialogResult"]
        fn notify_dialog_result(self: Pin<&mut Self>, id: &QString, payload: &QString);
        #[qinvokable]
        #[cxx_name = "notifyWebChannelResult"]
        fn notify_web_channel_result(self: Pin<&mut Self>, id: &QString, payload: &QString);
    }

    extern "RustQt" {
        #[qsignal]
        #[cxx_name = "rustEvent"]
        fn rust_event(self: Pin<&mut Page>, name: QString, payload: QString);
        #[qsignal]
        #[cxx_name = "invokeResult"]
        fn invoke_result(self: Pin<&mut Page>, id: QString, envelope: QString);
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type Page = super::PageRust;
        #[qinvokable]
        #[cxx_name = "startUrl"]
        fn start_url(&self) -> QString;
        #[qinvokable]
        #[cxx_name = "isDev"]
        fn is_dev(&self) -> bool;
        #[qinvokable]
        fn config(&self) -> QString;
        #[qinvokable]
        fn invoke(self: Pin<&mut Self>, cmd: &QString, args: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "emitEvent"]
        fn emit_event(self: Pin<&mut Self>, name: &QString, payload: &QString);
        #[qinvokable]
        #[cxx_name = "frontendReady"]
        fn frontend_ready(self: Pin<&mut Self>);
    }

    impl cxx_qt::Threading for Backend {}
    impl cxx_qt::Initialize for Backend {}
    impl cxx_qt::Threading for Page {}
    impl cxx_qt::Initialize for Page {}
}

#[derive(Default)]
pub struct BackendRust;

#[derive(Default)]
pub struct PageRust;

const RESERVED: &[&str] = &["app://", "window://", "tray://", "dialog://", "webchannel://", "deep-link://", "log://"];

static PAGE: OnceLock<CxxQtThread<qobject::Page>> = OnceLock::new();
static NEXT_INVOKE: AtomicU64 = AtomicU64::new(1);

pub(crate) fn send_to_page(name: String, json: String) {
    let Some(page) = PAGE.get() else { return };
    let label = name.clone();
    if let Err(e) = page.queue(move |page| page.rust_event(QString::from(&name), QString::from(&json))) {
        eprintln!("[app] cannot queue event `{label}`: {e}");
    }
}

impl cxx_qt::Initialize for qobject::Backend {
    fn initialize(self: Pin<&mut Self>) {
        let handle = AppHandle::new(self.qt_thread());
        app::set_global_handle(handle.clone());
        handle.emit("app://boot", json!({ "stage": "qt-initialized" }));
    }
}

impl cxx_qt::Initialize for qobject::Page {
    fn initialize(self: Pin<&mut Self>) { let _ = PAGE.set(self.qt_thread()); }
}

impl qobject::Backend {
    pub fn start_url(&self) -> QString { QString::from(&resolve_start_url()) }

    pub fn is_dev(&self) -> bool { is_dev_mode() }

    pub fn config(&self) -> QString { QString::from(config::raw()) }

    pub fn csp_script(&self) -> QString {
        let script =
            if is_dev_mode() && dev_url_override().is_some() { config::active_csp().map(|policy| csp_source(&allow_channel_client(&policy))) } else { None };
        QString::from(&script.unwrap_or_default())
    }

    pub fn user_scripts(&self) -> QString { QString::from(&crate::user_scripts_json()) }

    pub fn web_storage(&self) -> QString { QString::from(&config::web_storage().to_string()) }

    pub fn tray_icon_url(&self) -> QString {
        let url = config::tray_icon_url();
        if url.is_empty() {
            eprintln!("[tray] no icon: set app.trayIcon.iconPath or bundle.icon");
        }
        QString::from(&url)
    }

    pub fn notify(&self, title: &QString, body: &QString) -> bool { crate::notify(&title.to_string(), &body.to_string()) }

    pub fn injects_scripts(&self) -> bool { !resolve_start_url().starts_with("xiao://") }

    pub fn is_app_url(&self, url: &QString) -> bool { is_app_url(&url.to_string()) }

    pub fn open_external(&self, url: &QString) -> bool {
        let url = url.to_string();
        let scheme = url.split_once(':').map(|(scheme, _)| scheme.to_ascii_lowercase()).unwrap_or_default();
        if !matches!(scheme.as_str(), "http" | "https" | "mailto") {
            app::trace!("[navigation] not opening {url} outside the app");
            return false;
        }
        app::trace!("[navigation] opening {url} in the system browser");
        crate::open_url(&url)
    }

    pub fn page_leaving(self: Pin<&mut Self>) { app::detach_frontend(); }

    pub fn restart_app(self: Pin<&mut Self>) { AppHandle::new(self.qt_thread()).restart(); }

    pub fn notify_tray_event(self: Pin<&mut Self>, name: &QString, payload: &QString) { self.forward("tray://", name, payload); }

    pub fn notify_window_event(self: Pin<&mut Self>, name: &QString, payload: &QString) { self.forward("window://", name, payload); }

    pub fn notify_dialog_result(self: Pin<&mut Self>, id: &QString, payload: &QString) { self.forward("dialog://result/", id, payload); }

    pub fn notify_web_channel_result(self: Pin<&mut Self>, id: &QString, payload: &QString) { self.forward("webchannel://result/", id, payload); }

    fn forward(self: Pin<&mut Self>, prefix: &str, name: &QString, payload: &QString) {
        let handle = AppHandle::new(self.qt_thread());
        handle.emit_json(&format!("{prefix}{name}"), payload.to_string());
    }
}

impl qobject::Page {
    pub fn start_url(&self) -> QString { QString::from(&resolve_start_url()) }

    pub fn is_dev(&self) -> bool { is_dev_mode() }

    pub fn config(&self) -> QString { QString::from(config::raw()) }

    pub fn invoke(self: Pin<&mut Self>, cmd: &QString, args: &QString) -> QString {
        let cmd = cmd.to_string();
        let args = parse_json_object(&args.to_string());
        app::trace!("[invoke] {cmd} {args}");

        let Some(app) = app::global_handle().cloned() else {
            return QString::from(&envelope(Err("the app is still starting".to_owned())).to_string());
        };
        let Some(command) = commands::lookup(&cmd) else {
            return QString::from(&envelope(Err(format!("unknown command `{cmd}`"))).to_string());
        };

        if !command.on_worker {
            return QString::from(&envelope((command.handler)(&app, args)).to_string());
        }

        let id = NEXT_INVOKE.fetch_add(1, Ordering::Relaxed).to_string();
        let handler = command.handler.clone();
        let thread = self.qt_thread();
        let reply_id = id.clone();
        let spawned = std::thread::Builder::new().name(format!("xiao:{cmd}")).spawn(move || {
            let result = envelope(handler(&app, args)).to_string();
            if let Err(e) = thread.queue(move |page| page.invoke_result(QString::from(&reply_id), QString::from(&result))) {
                eprintln!("[invoke] cannot deliver a result: {e}");
            }
        });

        match spawned {
            Ok(_) => QString::from(&json!({ "ok": true, "pending": id }).to_string()),
            Err(e) => QString::from(&envelope(Err(format!("cannot start a thread for `{cmd}`: {e}"))).to_string()),
        }
    }

    pub fn emit_event(self: Pin<&mut Self>, name: &QString, payload: &QString) {
        let name = name.to_string();
        if RESERVED.iter().any(|prefix| name.starts_with(prefix)) {
            app::trace!("[emit] the page may not emit {name}, ignored");
            return;
        }
        if let Some(app) = app::global_handle() {
            app.emit_json(&name, payload.to_string());
        }
    }

    pub fn frontend_ready(self: Pin<&mut Self>) {
        if let Some(app) = app::global_handle() {
            app.attach_frontend();
        }
    }
}

fn envelope(result: Result<Value, String>) -> Value {
    match result {
        Ok(data) => json!({ "ok": true, "data": data }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

fn allow_channel_client(policy: &str) -> String {
    let mut directives: Vec<String> = policy.split(';').map(|directive| directive.trim().to_owned()).filter(|directive| !directive.is_empty()).collect();
    let named = |directives: &[String], name: &str| directives.iter().position(|directive| directive == name || directive.starts_with(&format!("{name} ")));
    if let Some(at) = named(&directives, "script-src").or_else(|| named(&directives, "default-src")) {
        directives[at].push_str(" xiao://localhost");
    }
    directives.join("; ")
}

fn csp_source(policy: &str) -> String {
    let policy = serde_json::to_string(policy).unwrap_or_else(|_| "\"\"".to_owned());
    format!(
        "(function () {{
  function install() {{
    var meta = document.createElement('meta');
    meta.httpEquiv = 'Content-Security-Policy';
    meta.content = {policy};
    (document.head || document.documentElement).appendChild(meta);
  }}
  if (document.head) {{ install(); return; }}
  new MutationObserver(function (_, observer) {{
    if (document.head) {{ install(); observer.disconnect(); }}
  }}).observe(document.documentElement || document, {{ childList: true, subtree: true }});
}})();"
    )
}

fn parse_json_object(raw: &str) -> Value { serde_json::from_str(raw).unwrap_or_else(|_| json!({})) }

pub(crate) fn is_dev_mode() -> bool {
    static DEV: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DEV.get_or_init(|| cfg!(debug_assertions) && (std::env::args_os().any(|arg| arg == "--dev") || std::env::var("XIAO_DEV").is_ok()))
}

fn dev_url_override() -> Option<String> { if cfg!(debug_assertions) { std::env::var("XIAO_DEV_URL").ok() } else { None } }

fn dev_server_url() -> String {
    dev_url_override().unwrap_or_else(|| config::config()["build"]["devUrl"].as_str().unwrap_or("http://localhost:1420").to_owned())
}

fn resolve_start_url() -> String {
    if let Some(url) = dev_url_override() {
        return url;
    }

    match config::frontend_url() {
        Some(url) => url,
        None => {
            eprintln!("[startUrl] no bundled frontend, falling back to the dev server");
            dev_server_url()
        }
    }
}

fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    Some(format!("{}://{}", scheme.to_ascii_lowercase(), authority.to_ascii_lowercase()))
}

pub(crate) fn is_app_url(url: &str) -> bool {
    let Some(origin) = origin_of(url) else { return false };
    if origin == "xiao://localhost" {
        return true;
    }
    cfg!(debug_assertions) && origin_of(&dev_server_url()).is_some_and(|dev| dev == origin)
}
