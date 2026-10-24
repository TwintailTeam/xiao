use std::fmt::{self, Write};
use std::mem::MaybeUninit;

#[repr(C)]
pub struct QString {
    _space: MaybeUninit<[usize; 3]>,
}

unsafe impl cxx::ExternType for QString {
    type Id = cxx::type_id!("QString");
    type Kind = cxx::kind::Trivial;
}

impl QString {
    fn as_slice(&self) -> &[u16] {
        let mut len = 0;
        let data = unsafe { crate::ffi::qstring_utf16(self, &mut len) };
        if len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(data, len) } }
    }
}

impl Drop for QString {
    fn drop(&mut self) { unsafe { crate::ffi::qstring_drop(self) } }
}

impl From<&str> for QString {
    fn from(text: &str) -> Self {
        let mut string = MaybeUninit::<Self>::uninit();
        unsafe {
            crate::ffi::qstring_init(string.as_mut_ptr(), text.as_ptr(), text.len());
            string.assume_init()
        }
    }
}

impl From<&String> for QString {
    fn from(text: &String) -> Self { Self::from(text.as_str()) }
}

impl fmt::Display for QString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if f.width().is_some() || f.precision().is_some() {
            return f.pad(&String::from_utf16_lossy(self.as_slice()));
        }
        for c in char::decode_utf16(self.as_slice().iter().copied()) {
            f.write_char(c.unwrap_or(char::REPLACEMENT_CHARACTER))?;
        }
        Ok(())
    }
}
