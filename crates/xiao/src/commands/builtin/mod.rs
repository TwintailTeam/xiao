mod clipboard;
mod deep_link;
mod dialog;
mod logging;
mod opener;
mod path;
mod tray;
mod webchannel;
mod window;

pub(super) fn registry() -> super::Registry {
    crate::commands![
        path::Path,
        window::Window,
        tray::Tray,
        dialog::Dialog,
        webchannel::WebChannel,
        opener::Opener,
        clipboard::Clipboard,
        deep_link::DeepLink,
        logging::Log
    ]
}
