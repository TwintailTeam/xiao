use serde::Serialize;
use std::path::Path;

const TEXT: i32 = 1;
const HTML: i32 = 2;
const IMAGE: i32 = 4;

const UNREACHABLE: &str = "the clipboard is only reachable from the thread Qt runs on";

/// Which formats the clipboard currently holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ClipboardKinds {
    pub text: bool,
    pub html: bool,
    pub image: bool,
}

/// The system clipboard, from [`AppHandle::clipboard`](crate::AppHandle::clipboard).
/// Every method must be called on the Qt thread.
pub struct Clipboard(());

impl Clipboard {
    pub(crate) fn new() -> Self { Self(()) }

    pub fn kinds(&self) -> Result<ClipboardKinds, String> {
        let kinds = crate::clipboard_kinds();
        if kinds < 0 {
            return Err(UNREACHABLE.to_owned());
        }
        Ok(ClipboardKinds { text: kinds & TEXT != 0, html: kinds & HTML != 0, image: kinds & IMAGE != 0 })
    }

    pub fn write_text(&self, text: &str) -> Result<(), String> {
        crate::app::trace!("[clipboard] write text ({} bytes)", text.len());
        done(crate::clipboard_write_text(text))
    }

    pub fn read_text(&self) -> Result<String, String> {
        if !self.kinds()?.text {
            return Err("the clipboard holds no text".to_owned());
        }
        crate::clipboard_read_text().ok_or_else(|| UNREACHABLE.to_owned())
    }

    /// Writes HTML, with `plain` as the text for applications that don't take
    /// HTML. Without `plain`, the markup itself is used.
    pub fn write_html(&self, html: &str, plain: Option<&str>) -> Result<(), String> {
        crate::app::trace!("[clipboard] write html ({} bytes)", html.len());
        done(crate::clipboard_write_html(html, plain))
    }

    pub fn read_html(&self) -> Result<String, String> {
        if !self.kinds()?.html {
            return Err("the clipboard holds no html".to_owned());
        }
        crate::clipboard_read_html().ok_or_else(|| UNREACHABLE.to_owned())
    }

    /// Writes a base64 encoded image, optionally as a `data:` URL, in any format Qt can decode.
    pub fn write_image(&self, base64: &str) -> Result<(), String> {
        crate::app::trace!("[clipboard] write image ({} base64 bytes)", base64.len());
        match crate::clipboard_write_image(base64) {
            true => Ok(()),
            false => Err("that is not an image Qt can decode".to_owned()),
        }
    }

    /// Writes the image file at `path`.
    pub fn write_image_file(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        crate::app::trace!("[clipboard] write image {}", path.display());
        match crate::clipboard_write_image_file(path) {
            true => Ok(()),
            false => Err(format!("{} is not an image Qt can read", path.display())),
        }
    }

    /// Reads the clipboard's image as a base64 encoded PNG.
    pub fn read_image(&self) -> Result<String, String> {
        if !self.kinds()?.image {
            return Err("the clipboard holds no image".to_owned());
        }
        crate::clipboard_read_image().ok_or_else(|| "cannot encode the clipboard image as PNG".to_owned())
    }

    pub fn clear(&self) -> Result<(), String> {
        crate::app::trace!("[clipboard] clear");
        done(crate::clipboard_clear())
    }
}

fn done(ok: bool) -> Result<(), String> {
    match ok {
        true => Ok(()),
        false => Err(UNREACHABLE.to_owned()),
    }
}
