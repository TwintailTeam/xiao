#![windows_subsystem = "windows"]

mod inflate;

use inflate::Inflate;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Component, Path};

const MAGIC: &[u8; 8] = b"XIAOSFX1";
const MARKER: &str = ".xiao-sfx";
const BUFFER: usize = 1 << 20;

struct Payload {
    offset: u64,
    len: u64,
    key: String,
    identifier: String,
    version: String,
    exe: String,
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code as i32),
        Err(e) => {
            message_box(&e);
            std::process::exit(1)
        }
    }
}

fn run() -> Result<u32, String> {
    let me = std::env::current_exe().map_err(|e| format!("cannot locate this program: {e}"))?;
    let mut file = File::open(&me).map_err(|e| format!("{}: {e}", me.display()))?;
    let payload = read_payload(&mut file).ok_or_else(|| format!("{} is damaged: the app inside it is missing", me.display()))?;

    let base = std::env::var_os("LOCALAPPDATA").filter(|dir| !dir.is_empty()).ok_or("LOCALAPPDATA is not set")?;
    let base = Path::new(&base).join("Programs").join(&payload.identifier);
    let name = format!("{}-{}", payload.version, payload.key);
    let dir = base.join(&name);

    if !dir.join(MARKER).is_file() {
        extract(&mut file, &payload, &base, &dir)?;
    }
    drop(file);
    remove_stale(&base, &name, &payload.exe);

    let app = dir.join(&payload.exe);
    let mut sfx = app.clone().into_os_string();
    sfx.push("|");
    sfx.push(&me);
    unsafe { std::env::set_var("XIAO_SFX", sfx) };
    launch(&app)
}

fn read_payload(file: &mut File) -> Option<Payload> {
    let pe = u64::from(u32::from_le_bytes(array_at(file, 0x3c)?));
    if &array_at::<4>(file, pe)? != b"PE\0\0" {
        return None;
    }
    let sections = u64::from(u16::from_le_bytes(array_at(file, pe + 6)?));
    let table = pe + 24 + u64::from(u16::from_le_bytes(array_at(file, pe + 20)?));

    let mut end = 0;
    for index in 0..sections {
        let at = table + index * 40;
        let size = u64::from(u32::from_le_bytes(array_at(file, at + 16)?));
        let pointer = u64::from(u32::from_le_bytes(array_at(file, at + 20)?));
        if pointer > 0 {
            end = end.max(pointer + size);
        }
    }

    file.seek(SeekFrom::Start(end)).ok()?;
    if &array::<8>(file)? != MAGIC {
        return None;
    }
    let len = u64::from_le_bytes(array(file)?);
    let key = array::<6>(file)?.iter().map(|byte| format!("{byte:02x}")).collect();
    let identifier = text(file)?;
    let version = text(file)?;
    let exe = text(file)?;
    let offset = file.stream_position().ok()?;
    [&identifier, &version, &exe].iter().all(|part| single_component(part)).then_some(Payload { offset, len, key, identifier, version, exe })
}

fn array_at<const N: usize>(file: &mut File, at: u64) -> Option<[u8; N]> {
    file.seek(SeekFrom::Start(at)).ok()?;
    array(file)
}

fn array<const N: usize>(reader: &mut impl Read) -> Option<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

fn text(reader: &mut impl Read) -> Option<String> {
    let mut bytes = vec![0; usize::from(u16::from_le_bytes(array(reader)?))];
    reader.read_exact(&mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

fn single_component(part: &str) -> bool {
    let mut components = Path::new(part).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn extract(file: &mut File, payload: &Payload, base: &Path, dir: &Path) -> Result<(), String> {
    let partial = base.join(format!(".{}-{}", payload.key, std::process::id()));
    let _ = std::fs::remove_dir_all(&partial);

    let unpacked = unpack(file, payload, &partial).and_then(|()| std::fs::write(partial.join(MARKER), b""));
    if let Err(e) = unpacked {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(format!("cannot unpack the app into {}: {e}", base.display()));
    }

    if std::fs::rename(&partial, dir).is_ok() {
        return Ok(());
    }
    if dir.join(MARKER).is_file() {
        let _ = std::fs::remove_dir_all(&partial);
        return Ok(());
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&partial, dir).map_err(|e| {
        let _ = std::fs::remove_dir_all(&partial);
        format!("cannot move the app into {}: {e}", dir.display())
    })
}

fn unpack(file: &mut File, payload: &Payload, dest: &Path) -> std::io::Result<()> {
    let invalid = || std::io::Error::new(std::io::ErrorKind::InvalidData, "the app inside this program is damaged");

    file.seek(SeekFrom::Start(payload.offset))?;
    let mut stream = Inflate::new(BufReader::with_capacity(BUFFER, file.take(payload.len)));

    loop {
        let len = usize::from(u16::from_le_bytes(array(&mut stream).ok_or_else(invalid)?));
        if len == 0 {
            let mut rest = [0; 1];
            return if stream.read(&mut rest)? == 0 { Ok(()) } else { Err(invalid()) };
        }
        let mut relative = vec![0; len];
        stream.read_exact(&mut relative)?;
        let relative = String::from_utf8(relative).map_err(|_| invalid())?;
        if !Path::new(&relative).components().all(|part| matches!(part, Component::Normal(_))) {
            return Err(invalid());
        }
        let size = u64::from_le_bytes(array(&mut stream).ok_or_else(invalid)?);

        let target = dest.join(&relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = BufWriter::with_capacity(BUFFER, File::create(&target)?);
        if std::io::copy(&mut (&mut stream).take(size), &mut out)? != size {
            return Err(invalid());
        }
        out.flush()?;
    }
}

fn remove_stale(base: &Path, current: &str, exe: &str) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == current || !path.is_dir() {
            continue;
        }
        let idle = if is_partial(&name) {
            entry.metadata().and_then(|meta| meta.modified()).ok().and_then(|time| time.elapsed().ok()).is_some_and(|age| age.as_secs() > 3600)
        } else if path.join(MARKER).is_file() {
            match std::fs::OpenOptions::new().write(true).share_mode(0).open(path.join(exe)) {
                Ok(_) => true,
                Err(e) => e.kind() == std::io::ErrorKind::NotFound,
            }
        } else {
            false
        };
        if idle {
            let _ = std::fs::remove_file(path.join(MARKER));
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

fn is_partial(name: &str) -> bool {
    let Some((key, pid)) = name.strip_prefix('.').and_then(|rest| rest.split_once('-')) else { return false };
    key.len() == 12 && key.bytes().all(|byte| byte.is_ascii_hexdigit()) && !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit())
}

fn launch(app: &Path) -> Result<u32, String> {
    let application: Vec<u16> = app.as_os_str().encode_wide().chain([0]).collect();
    let mut command = vec![u16::from(b'"')];
    command.extend(app.as_os_str().encode_wide());
    command.push(u16::from(b'"'));
    command.extend(arguments());
    command.push(0);

    unsafe {
        let mut parent: ffi::StartupInfo = std::mem::zeroed();
        ffi::GetStartupInfoW(&mut parent);
        let mut startup: ffi::StartupInfo = std::mem::zeroed();
        startup.cb = size_of::<ffi::StartupInfo>() as u32;
        startup.flags = parent.flags & (ffi::STARTF_USESHOWWINDOW | ffi::STARTF_USESTDHANDLES);
        startup.show_window = parent.show_window;
        startup.std_input = parent.std_input;
        startup.std_output = parent.std_output;
        startup.std_error = parent.std_error;

        let mut info: ffi::ProcessInformation = std::mem::zeroed();
        let inherit = i32::from(startup.flags & ffi::STARTF_USESTDHANDLES != 0);
        let created = ffi::CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            inherit,
            0,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut info,
        );
        if created == 0 {
            return Err(format!("cannot start {}: {}", app.display(), std::io::Error::last_os_error()));
        }
        ffi::CloseHandle(info.thread);
        ffi::WaitForSingleObject(info.process, ffi::INFINITE);
        let mut code = 0;
        ffi::GetExitCodeProcess(info.process, &mut code);
        ffi::CloseHandle(info.process);
        Ok(code)
    }
}

fn arguments() -> Vec<u16> {
    let line = unsafe {
        let line = ffi::GetCommandLineW();
        let len = (0..).take_while(|&at| *line.add(at) != 0).count();
        std::slice::from_raw_parts(line, len)
    };
    let mut quoted = false;
    let end = line
        .iter()
        .position(|&unit| {
            if unit == u16::from(b'"') {
                quoted = !quoted;
            }
            !quoted && (unit == u16::from(b' ') || unit == u16::from(b'\t'))
        })
        .unwrap_or(line.len());
    line[end..].to_vec()
}

fn message_box(message: &str) {
    let caption = std::env::current_exe().ok().and_then(|exe| exe.file_stem().map(|stem| stem.to_string_lossy().into_owned())).unwrap_or_default();
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain([0]).collect() };
    unsafe { ffi::MessageBoxW(std::ptr::null_mut(), wide(message).as_ptr(), wide(&caption).as_ptr(), ffi::MB_ICONERROR) };
}

mod ffi {
    use std::ffi::c_void;

    pub const STARTF_USESHOWWINDOW: u32 = 0x1;
    pub const STARTF_USESTDHANDLES: u32 = 0x100;
    pub const INFINITE: u32 = u32::MAX;
    pub const MB_ICONERROR: u32 = 0x10;

    #[repr(C)]
    pub struct StartupInfo {
        pub cb: u32,
        pub reserved: *mut u16,
        pub desktop: *mut u16,
        pub title: *mut u16,
        pub x: u32,
        pub y: u32,
        pub width: u32,
        pub height: u32,
        pub columns: u32,
        pub rows: u32,
        pub fill_attribute: u32,
        pub flags: u32,
        pub show_window: u16,
        pub reserved2_len: u16,
        pub reserved2: *mut u8,
        pub std_input: *mut c_void,
        pub std_output: *mut c_void,
        pub std_error: *mut c_void,
    }

    #[repr(C)]
    pub struct ProcessInformation {
        pub process: *mut c_void,
        pub thread: *mut c_void,
        pub process_id: u32,
        pub thread_id: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetCommandLineW() -> *const u16;
        pub fn GetStartupInfoW(info: *mut StartupInfo);
        pub fn CreateProcessW(
            application: *const u16,
            command: *mut u16,
            process_attributes: *const c_void,
            thread_attributes: *const c_void,
            inherit: i32,
            flags: u32,
            environment: *const c_void,
            directory: *const u16,
            startup: *const StartupInfo,
            info: *mut ProcessInformation,
        ) -> i32;
        pub fn WaitForSingleObject(handle: *mut c_void, millis: u32) -> u32;
        pub fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
        pub fn CloseHandle(handle: *mut c_void) -> i32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn MessageBoxW(window: *mut c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
}
