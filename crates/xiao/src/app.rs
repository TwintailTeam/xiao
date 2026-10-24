use crate::bridge::qobject::Backend;
use crate::qstring::QString;
use cxx_qt::CxxQtThread;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static ATTACHED: AtomicBool = AtomicBool::new(false);
static PENDING: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

static RESTART: AtomicBool = AtomicBool::new(false);

pub(crate) fn exit_code() -> i32 { EXIT_CODE.load(Ordering::Acquire) }

pub(crate) fn restart_requested() -> bool { RESTART.load(Ordering::Acquire) }

macro_rules! trace {
    ($($arg:tt)*) => {
        if crate::bridge::is_dev_mode() {
            println!($($arg)*);
        }
    };
}

pub(crate) use trace;

/// A handle to the running app, passed to every command and listener.
/// Cheap to clone and usable from any thread.
#[derive(Clone)]
pub struct AppHandle {
    thread: CxxQtThread<Backend>,
}

impl AppHandle {
    pub(crate) fn new(thread: CxxQtThread<Backend>) -> Self { Self { thread } }

    /// Emits `event` to Rust listeners and the page. Events emitted before the
    /// page is ready are delivered once it is.
    pub fn emit<T: Serialize>(&self, event: &str, payload: T) {
        let json = serde_json::to_string(&payload).unwrap_or_else(|_| "null".to_owned());
        self.emit_json(event, json);
    }

    /// Calls `handler` for every `event`, whether the page, the shell or Rust emitted it.
    pub fn listen<F>(&self, event: &str, handler: F) -> EventId
    where F: Fn(&AppHandle, Value) + Send + Sync + 'static {
        subscribe(event, false, Arc::new(handler))
    }

    /// Like [`listen`](Self::listen), for the next `event` only.
    pub fn once<F>(&self, event: &str, handler: F) -> EventId
    where F: Fn(&AppHandle, Value) + Send + Sync + 'static {
        subscribe(event, true, Arc::new(handler))
    }

    /// Removes a listener. Unknown or already finished ids are ignored.
    pub fn unlisten(&self, id: EventId) { SUBSCRIPTIONS.lock().unwrap().retain(|s| s.id != id.0); }

    /// The app's `xiao.conf.json`.
    pub fn config(&self) -> &'static Value { crate::config::config() }

    /// Manages `value` for the app's lifetime. Returns `false`, dropping
    /// `value`, if a value of that type is already managed.
    pub fn manage<T: Send + Sync + 'static>(&self, value: T) -> bool { crate::state::manage(value) }

    /// The managed value of type `T`.
    ///
    /// # Panics
    ///
    /// If no value of type `T` is managed.
    pub fn state<T: Send + Sync + 'static>(&self) -> crate::State<'static, T> { crate::state::state::<T>() }

    /// The managed value of type `T`, if there is one.
    pub fn try_state<T: Send + Sync + 'static>(&self) -> Option<crate::State<'static, T>> { crate::state::try_state::<T>() }

    /// Runs `task` on the Qt thread.
    pub fn run_on_main_thread<F>(&self, task: F)
    where F: FnOnce() + Send + 'static {
        if let Err(e) = self.thread.queue(move |_| task()) {
            eprintln!("[app] cannot queue a task for the Qt thread: {e}");
        }
    }

    /// Closes the window and ends the app. [`Shell::on_exit`](crate::Shell::on_exit)
    /// runs, then the process exits with `code`.
    pub fn exit(&self, code: i32) {
        EXIT_CODE.store(code, Ordering::Release);
        self.window().destroy();
    }

    /// Ends the app like [`exit`](Self::exit) with code 0, then starts it
    /// again with the same arguments.
    pub fn restart(&self) {
        RESTART.store(true, Ordering::Release);
        self.exit(0);
    }

    /// Standard and app-specific directories.
    pub fn path(&self) -> crate::path::PathResolver { crate::path::PathResolver::new() }

    /// The app's window.
    pub fn window(&self) -> crate::window::WindowHandle<'_> { crate::window::WindowHandle { app: self } }

    /// The tray icon.
    pub fn tray(&self) -> crate::tray::TrayHandle<'_> { crate::tray::TrayHandle { app: self } }

    /// Opens URLs, files and folders with the desktop's applications.
    pub fn opener(&self) -> crate::opener::Opener { crate::opener::Opener::new() }

    /// The system clipboard.
    pub fn clipboard(&self) -> crate::clipboard::Clipboard { crate::clipboard::Clipboard::new() }

    /// URL schemes that open the app.
    pub fn deep_link(&self) -> crate::deep_link::DeepLinkHandle<'_> { crate::deep_link::DeepLinkHandle { app: self } }

    /// Native file and message dialogs.
    pub fn dialog(&self) -> crate::dialog::DialogHandle<'_> { crate::dialog::DialogHandle { app: self } }

    /// Lower-level access to the page and its web channel.
    pub fn webchannel(&self) -> crate::webchannel::WebChannelHandle<'_> { crate::webchannel::WebChannelHandle { app: self } }

    pub(crate) fn emit_json(&self, event: &str, json: String) {
        let payload: Value = serde_json::from_str(&json).unwrap_or(Value::Null);
        crate::window::record(event, &payload);
        dispatch_event(self, event, &payload);

        let name = event.to_owned();

        if !ATTACHED.load(Ordering::Acquire) {
            trace!("[emit] {name} {json} (buffered)");
            PENDING.lock().unwrap().push((name, json));
            return;
        }

        trace!("[emit] {name} {json}");
        self.send(name, json);
    }

    pub(crate) fn window_command(&self, action: &str, args: Value) {
        self.queue("window", action, args, |qobject, action, args| qobject.window_command(action, args));
    }

    pub(crate) fn tray_command(&self, action: &str, args: Value) {
        self.queue("tray", action, args, |qobject, action, args| qobject.tray_command(action, args));
    }

    pub(crate) fn webchannel_command(&self, action: &str, args: Value) {
        self.queue("webchannel", action, args, |qobject, action, args| qobject.web_channel_command(action, args));
    }

    pub(crate) fn dialog_command(&self, args: Value) {
        let args = args.to_string();
        if let Err(e) = self.thread.queue(move |qobject| qobject.dialog_command(QString::from(&args))) {
            eprintln!("[app] cannot queue dialog command: {e}");
        }
    }

    fn queue<F>(&self, what: &str, action: &str, args: Value, apply: F)
    where F: FnOnce(core::pin::Pin<&mut Backend>, QString, QString) + Send + 'static {
        let label = action.to_owned();
        let action = action.to_owned();
        let args = args.to_string();

        let queued = self.thread.queue(move |qobject| apply(qobject, QString::from(&action), QString::from(&args)));
        if let Err(e) = queued {
            eprintln!("[app] cannot queue {what} command `{label}`: {e}");
        }
    }

    fn send(&self, name: String, json: String) { crate::bridge::send_to_page(name, json); }

    pub(crate) fn attach_frontend(&self) {
        if ATTACHED.swap(true, Ordering::AcqRel) {
            return;
        }

        let buffered: Vec<_> = std::mem::take(&mut *PENDING.lock().unwrap());
        if !buffered.is_empty() {
            trace!("[emit] flushing {} buffered event(s)", buffered.len());
        }
        for (name, json) in buffered {
            self.send(name, json);
        }
        self.emit("app://ready", serde_json::json!({ "pid": std::process::id(), "renderer": "QtWebEngine", "version": crate::config::version() }));
    }
}

pub(crate) fn is_attached() -> bool { ATTACHED.load(Ordering::Acquire) }

pub(crate) fn detach_frontend() { ATTACHED.store(false, Ordering::Release); }

/// A listener registered through [`Shell::listeners`](crate::Shell::listeners).
pub type EventHandler = Box<dyn Fn(&AppHandle, Value) + Send + Sync>;

/// Identifies a listener for [`AppHandle::unlisten`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EventId(u64);

static GLOBAL: OnceLock<AppHandle> = OnceLock::new();

pub(crate) fn set_global_handle(handle: AppHandle) { let _ = GLOBAL.set(handle); }

/// The running app's handle, once Qt has started.
pub fn global_handle() -> Option<&'static AppHandle> { GLOBAL.get() }

pub(crate) fn wait_for_global_handle(timeout: std::time::Duration) -> Option<AppHandle> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Some(handle) = GLOBAL.get() {
            return Some(handle.clone());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    None
}

type SharedHandler = Arc<dyn Fn(&AppHandle, Value) + Send + Sync>;

struct Subscription {
    id: u64,
    event: String,
    once: bool,
    handler: SharedHandler,
}

static SUBSCRIPTIONS: Mutex<Vec<Subscription>> = Mutex::new(Vec::new());
static NEXT_SUBSCRIPTION: AtomicU64 = AtomicU64::new(1);

pub(crate) fn install_listeners(handlers: HashMap<&'static str, EventHandler>) {
    for (event, handler) in handlers {
        subscribe(event, false, Arc::from(handler));
    }
}

fn subscribe(event: &str, once: bool, handler: SharedHandler) -> EventId {
    let id = NEXT_SUBSCRIPTION.fetch_add(1, Ordering::Relaxed);
    SUBSCRIPTIONS.lock().unwrap().push(Subscription { id, event: event.to_owned(), once, handler });
    EventId(id)
}

pub(crate) fn dispatch_event(app: &AppHandle, name: &str, payload: &Value) -> usize {
    let matched: Vec<SharedHandler> = {
        let mut subscriptions = SUBSCRIPTIONS.lock().unwrap();
        let matched = subscriptions.iter().filter(|s| s.event == name).map(|s| Arc::clone(&s.handler)).collect();
        subscriptions.retain(|s| !(s.once && s.event == name));
        matched
    };

    for handler in &matched {
        handler(app, payload.clone());
    }
    matched.len()
}
