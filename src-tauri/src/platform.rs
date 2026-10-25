//! OS-specific bits kept in one place.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const PLATFORM: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "macos"
} else {
    "linux"
};

/// Steam lists the Rust dedicated server (258550) for Windows and Linux only.
pub const CAN_HOST_SERVER: bool = cfg!(any(windows, target_os = "linux"));

pub fn open_in_file_manager(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    Command::new(program).arg(path).spawn().map(|_| ())
}

/// Open the file manager with `path` selected.
pub fn reveal(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer wants exactly: /select,"C:\path with spaces\file"
        Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    open_in_file_manager(path.parent().unwrap_or(path))
}

/// The Rust game's process (the same name under Proton on Linux).
pub const RUST_CLIENT: &str = "RustClient.exe";

/// Hand a URL (steam://connect/...) to the program registered for it.
#[cfg(windows)]
pub fn open_url(url: &str) -> std::io::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(url));
    // SAFETY: NUL-terminated strings that outlive the call; no window handle needed.
    let result = unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
    // ShellExecute reports success with a value above 32.
    if result as isize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("nothing is registered to open {url}")))
    }
}

#[cfg(not(windows))]
pub fn open_url(url: &str) -> std::io::Result<()> {
    Command::new("xdg-open").arg(url).spawn().map(|_| ())
}

/// Bring a program's main window to the front (restored if minimized). False: no window found.
/// Works while DropLaunch is the window in front (the user just clicked in it).
#[cfg(windows)]
pub fn bring_to_front(pid: u32) -> bool {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE,
    };

    struct Search {
        pid: u32,
        found: HWND,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the Search passed to EnumWindows below, alive for the whole call.
        let search = unsafe { &mut *(data as *mut Search) };
        let mut pid = 0u32;
        // SAFETY: a window handle from EnumWindows; the out-pointer is a valid u32.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        // SAFETY: as above.
        let main = unsafe { IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() };
        if pid == search.pid && main {
            search.found = hwnd;
            return 0; // stop
        }
        1
    }

    let mut search = Search { pid, found: std::ptr::null_mut() };
    // SAFETY: the callback only touches `search` through the pointer, during this call.
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    if search.found.is_null() {
        return false;
    }
    // SAFETY: a window handle found above.
    unsafe {
        if IsIconic(search.found) != 0 {
            ShowWindow(search.found, SW_RESTORE);
        }
        SetForegroundWindow(search.found) != 0
    }
}

#[cfg(not(windows))]
pub fn bring_to_front(_pid: u32) -> bool {
    false
}

/// Ask a program's windows to close, like their X button (it may save or ask first).
#[cfg(windows)]
pub fn close_windows(pid: u32) -> bool {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE};

    struct Search {
        pid: u32,
        closed: bool,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the Search passed to EnumWindows below, alive for the whole call.
        let search = unsafe { &mut *(data as *mut Search) };
        let mut pid = 0u32;
        // SAFETY: a window handle from EnumWindows; the out-pointer is a valid u32.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        // SAFETY: as above; posting WM_CLOSE only asks the window to close.
        if pid == search.pid && unsafe { IsWindowVisible(hwnd) } != 0 && unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) } != 0 {
            search.closed = true;
        }
        1
    }

    let mut search = Search { pid, closed: false };
    // SAFETY: the callback only touches `search` through the pointer, during this call.
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    search.closed
}

#[cfg(not(windows))]
pub fn close_windows(_pid: u32) -> bool {
    false
}

/// Close the Rust game like its X button and wait for it to be gone (false: still running).
pub fn close_rust(wait: std::time::Duration) -> bool {
    for process in find_processes(RUST_CLIENT) {
        close_windows(process.pid);
    }
    let deadline = std::time::Instant::now() + wait;
    while std::time::Instant::now() < deadline {
        if find_processes(RUST_CLIENT).is_empty() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    find_processes(RUST_CLIENT).is_empty()
}

/// Free bytes on the drive holding `path` (None if unknown).
#[cfg(windows)]
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut free = 0u64;
    // SAFETY: NUL-terminated path; the out-pointers are a valid u64 or null.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

#[cfg(not(windows))]
pub fn free_space(_path: &Path) -> Option<u64> {
    None
}

/// Where DropLaunch shouldn't make its folders and downloads: a drive's root, or a folder that
/// holds lots of other things ("desktop", "downloads", "documents", "home": the user's own
/// folder). None: a folder of its own (or one that isn't known).
pub fn busy_place(dir: &Path) -> Option<&'static str> {
    if dir.parent().is_none() {
        return Some("drive");
    }
    busy_place_among(dir, &user_folders())
}

/// `dir` is one of `known`, the user's folders by kind.
fn busy_place_among(dir: &Path, known: &[(&'static str, PathBuf)]) -> Option<&'static str> {
    let key = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase();
    let dir = key(dir);
    known.iter().find(|(_, path)| key(path) == dir).map(|(kind, _)| *kind)
}

/// The user's Desktop, Downloads and Documents (where Windows really keeps them: they can be moved,
/// to OneDrive for one) and their own folder.
#[cfg(windows)]
fn user_folders() -> Vec<(&'static str, PathBuf)> {
    use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Profile};
    [("desktop", FOLDERID_Desktop), ("downloads", FOLDERID_Downloads), ("documents", FOLDERID_Documents), ("home", FOLDERID_Profile)]
        .into_iter()
        .filter_map(|(kind, id)| known_folder(&id).map(|path| (kind, path)))
        .collect()
}

#[cfg(not(windows))]
fn user_folders() -> Vec<(&'static str, PathBuf)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    vec![("desktop", home.join("Desktop")), ("downloads", home.join("Downloads")), ("documents", home.join("Documents")), ("home", home)]
}

#[cfg(windows)]
fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::SHGetKnownFolderPath;
    let mut raw: windows_sys::core::PWSTR = std::ptr::null_mut();
    // SAFETY: a valid folder id; Windows allocates the path, which is freed below either way.
    let status = unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut raw) };
    let path = (status == 0 && !raw.is_null()).then(|| {
        // SAFETY: on success `raw` is a NUL-terminated wide string.
        let len = (0..).take_while(|&i| unsafe { *raw.add(i) } != 0).count();
        PathBuf::from(std::ffi::OsString::from_wide(unsafe { std::slice::from_raw_parts(raw, len) }))
    });
    // SAFETY: allocated by SHGetKnownFolderPath (null is fine too).
    unsafe { CoTaskMemFree(raw.cast()) };
    path
}

/// Something on this PC has `port` open (`udp`: a UDP socket, else a TCP listener, on any
/// address, IPv4 or IPv6), read from Windows' own table of open ports. Nothing is opened to find
/// out: opening a port on every network makes Windows ask the firewall question for DropLaunch.
/// None: the table couldn't be read.
#[cfg(windows)]
pub fn port_in_use(udp: bool, port: u16) -> Option<bool> {
    use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID, MIB_UDP6TABLE_OWNER_PID, MIB_UDPTABLE_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER, UDP_TABLE_OWNER_PID,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    /// The table for one address family, in a buffer aligned for its rows.
    fn table(udp: bool, family: u32) -> Option<Vec<u64>> {
        let mut size = 0u32;
        let mut buf: Vec<u64> = Vec::new();
        for _ in 0..4 {
            // SAFETY: `buf` holds at least `size` bytes (or is empty with size 0, which only asks
            // for the size); the call writes no more than `size` bytes.
            let result = unsafe {
                if udp {
                    GetExtendedUdpTable(buf.as_mut_ptr().cast(), &mut size, 0, family, UDP_TABLE_OWNER_PID, 0)
                } else {
                    GetExtendedTcpTable(buf.as_mut_ptr().cast(), &mut size, 0, family, TCP_TABLE_OWNER_PID_LISTENER, 0)
                }
            };
            match result {
                0 => return Some(buf),
                // Grown since the size was asked: ask again with room to spare.
                ERROR_INSUFFICIENT_BUFFER => buf = vec![0u64; (size as usize).div_ceil(8) + 64],
                _ => return None,
            }
        }
        None
    }

    /// The ports in a table: its count, then its rows (`port` reads one row's local port).
    fn ports<T, R>(buf: &[u64], rows: impl Fn(&T) -> (u32, *const R), port: impl Fn(&R) -> u32) -> Vec<u16> {
        if buf.is_empty() {
            return Vec::new();
        }
        // SAFETY: the buffer holds a table of type T as Windows wrote it (aligned to 8 bytes),
        // with `count` rows after its header.
        unsafe {
            let (count, first) = rows(&*buf.as_ptr().cast::<T>());
            std::slice::from_raw_parts(first, count as usize).iter().map(|r| u16::from_be(port(r) as u16)).collect()
        }
    }

    let mut open = Vec::new();
    for family in [u32::from(AF_INET), u32::from(AF_INET6)] {
        let buf = table(udp, family)?;
        open.extend(match (udp, family == u32::from(AF_INET)) {
            (true, true) => ports(&buf, |t: &MIB_UDPTABLE_OWNER_PID| (t.dwNumEntries, t.table.as_ptr()), |r| r.dwLocalPort),
            (true, false) => ports(&buf, |t: &MIB_UDP6TABLE_OWNER_PID| (t.dwNumEntries, t.table.as_ptr()), |r| r.dwLocalPort),
            (false, true) => ports(&buf, |t: &MIB_TCPTABLE_OWNER_PID| (t.dwNumEntries, t.table.as_ptr()), |r| r.dwLocalPort),
            (false, false) => ports(&buf, |t: &MIB_TCP6TABLE_OWNER_PID| (t.dwNumEntries, t.table.as_ptr()), |r| r.dwLocalPort),
        });
    }
    Some(open.contains(&port))
}

#[cfg(not(windows))]
pub fn port_in_use(_udp: bool, _port: u16) -> Option<bool> {
    None
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A text value under HKEY_CURRENT_USER (read-only).
#[cfg(windows)]
pub fn user_registry_string(key: &str, value: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let (key, value) = (wide(key), wide(value));
    let mut buf = vec![0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: NUL-terminated names; the buffer and its byte size match.
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut bytes)
    };
    if status != 0 {
        return None;
    }
    buf.truncate((bytes as usize / 2).saturating_sub(1)); // drop the NUL
    Some(String::from_utf16_lossy(&buf))
}

/// A number value under HKEY_CURRENT_USER (read-only).
#[cfg(windows)]
pub fn user_registry_dword(key: &str, value: &str) -> Option<u32> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let (key, value) = (wide(key), wide(value));
    let mut data = 0u32;
    let mut bytes = 4u32;
    // SAFETY: NUL-terminated names; `data` is the 4 bytes a DWORD needs.
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut data as *mut u32).cast(), &mut bytes)
    };
    (status == 0).then_some(data)
}

/// Write a text value under HKEY_CURRENT_USER (the key is created when it's missing).
#[cfg(windows)]
pub fn set_user_registry_string(key: &str, value: &str, data: &str) -> std::io::Result<()> {
    use windows_sys::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let (key, value, data) = (wide(key), wide(value), wide(data));
    // SAFETY: NUL-terminated names; the byte count covers `data` including its NUL.
    let status = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(status as i32))
    }
}

/// Stop console programs from flashing a window when started from the GUI.
pub fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

#[cfg(windows)]
pub use win::{find_processes, launch_console, process_name, Launched, ProcInfo, ProcessHandle, ProcessJob};

#[cfg(windows)]
mod win {
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
        JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess, GetProcessTimes, InitializeProcThreadAttributeList,
        OpenProcess, QueryFullProcessImageNameW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
        CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROCESS_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE, PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTUPINFOEXW,
    };

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// A Windows job object holding a process and everything it starts, so the whole tree can
    /// be waited on or killed.
    pub struct ProcessJob(HANDLE);

    // SAFETY: the handle refers to a kernel object; the job APIs may be called from any thread.
    unsafe impl Send for ProcessJob {}
    unsafe impl Sync for ProcessJob {}

    impl ProcessJob {
        /// Everything in the job is killed when DropLaunch closes (used for SteamCMD).
        pub fn new() -> io::Result<Self> {
            Self::create(true)
        }

        /// Only tracks the processes: a server keeps running if DropLaunch closes.
        pub fn tracking() -> io::Result<Self> {
            Self::create(false)
        }

        fn create(kill_on_close: bool) -> io::Result<Self> {
            // SAFETY: plain Win32 calls; `info` is a zeroed POD struct of the size we pass.
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let job = ProcessJob(handle);
                if kill_on_close {
                    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                    let ok = SetInformationJobObject(
                        handle,
                        JobObjectExtendedLimitInformation,
                        &info as *const _ as *const _,
                        std::mem::size_of_val(&info) as u32,
                    );
                    if ok == 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(job)
            }
        }

        pub fn assign_pid(&self, pid: u32) -> io::Result<()> {
            // SAFETY: the process handle is closed right after use.
            unsafe {
                let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
                if process.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let result = self.assign(process);
                CloseHandle(process);
                result
            }
        }

        fn assign(&self, process: HANDLE) -> io::Result<()> {
            // SAFETY: both handles are valid for the duration of the call.
            if unsafe { AssignProcessToJobObject(self.0, process) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        pub fn active_processes(&self) -> u32 {
            // SAFETY: `info` is a POD out-parameter of the size we pass.
            unsafe {
                let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
                let ok = QueryInformationJobObject(
                    self.0,
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as *mut _,
                    std::mem::size_of_val(&info) as u32,
                    std::ptr::null_mut(),
                );
                if ok == 0 {
                    0
                } else {
                    info.ActiveProcesses
                }
            }
        }

        pub fn terminate(&self) {
            // SAFETY: valid job handle owned by self.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for ProcessJob {
        fn drop(&mut self) {
            // SAFETY: we own the handle (with KILL_ON_JOB_CLOSE this ends leftover processes).
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub struct Launched {
        pub shell_pid: u32,
        pub job: ProcessJob,
    }

    /// Run a script from `dir` in its own, visible console window, inside a tracking job.
    ///
    /// Uses CreateProcessW directly: std::process::Command always hands the child DropLaunch's
    /// own stdin/stdout (none for a GUI app, so the window would stay blank and `pause` wouldn't
    /// wait). It's created in the job (a process attribute), so it's in there before it can start
    /// anything itself; no starting it suspended and resuming it, which is what code injection
    /// looks like to antivirus software.
    pub fn launch_console(dir: &Path, script: &str, env: &[(&str, &str)]) -> io::Result<Launched> {
        let shell = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into());
        // ".\" because cmd won't run files from the current folder by bare name when
        // NoDefaultCurrentDirectoryInExePath is set; a relative path also avoids quoting trouble.
        let command = format!("\"{}\" /C .\\{script}", shell.to_string_lossy());
        let mut command_line = wide(OsStr::new(&command));
        let cwd = wide(dir.as_os_str());
        let environment = environment_block(env);
        let job = ProcessJob::tracking()?;

        // SAFETY: zeroed POD structs with cb set; the attribute list lives in `attributes` (sized
        // by the first call) and is deleted before it's freed; the job handle it points to and all
        // other buffers are NUL-terminated where needed and outlive CreateProcessW.
        unsafe {
            let mut size = 0usize;
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size);
            let mut attributes = vec![0u8; size];
            let list: LPPROC_THREAD_ATTRIBUTE_LIST = attributes.as_mut_ptr().cast();
            if InitializeProcThreadAttributeList(list, 1, 0, &mut size) == 0 {
                return Err(io::Error::last_os_error());
            }
            let jobs = [job.0];
            let ok = UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                jobs.as_ptr().cast(),
                std::mem::size_of_val(&jobs),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if ok == 0 {
                let err = io::Error::last_os_error();
                DeleteProcThreadAttributeList(list);
                return Err(err);
            }
            let mut si: STARTUPINFOEXW = std::mem::zeroed();
            si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            si.lpAttributeList = list;
            let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
            let ok = CreateProcessW(
                std::ptr::null(),
                command_line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                environment.as_ptr() as *const _,
                cwd.as_ptr(),
                &si.StartupInfo,
                &mut pi,
            );
            let err = (ok == 0).then(io::Error::last_os_error);
            DeleteProcThreadAttributeList(list);
            if let Some(err) = err {
                return Err(err);
            }
            CloseHandle(pi.hProcess);
            CloseHandle(pi.hThread);
            Ok(Launched { shell_pid: pi.dwProcessId, job })
        }
    }

    /// DropLaunch's environment plus `extra`, as a sorted UTF-16 block.
    fn environment_block(extra: &[(&str, &str)]) -> Vec<u16> {
        let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(k, _)| !extra.iter().any(|(e, _)| k.to_string_lossy().eq_ignore_ascii_case(e)))
            .collect();
        vars.extend(extra.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))));
        vars.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
        let mut block = Vec::new();
        for (k, v) in vars {
            block.extend(k.encode_wide());
            block.push(b'=' as u16);
            block.extend(v.encode_wide());
            block.push(0);
        }
        block.push(0);
        block
    }

    /// A process DropLaunch keeps an eye on (a running Rust server).
    pub struct ProcessHandle {
        handle: HANDLE,
        pub pid: u32,
    }

    // SAFETY: a process handle may be used from any thread.
    unsafe impl Send for ProcessHandle {}

    impl ProcessHandle {
        pub fn open(pid: u32) -> Option<Self> {
            // SAFETY: plain Win32 call; the handle is owned (and closed) by Self.
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            (!handle.is_null()).then_some(Self { handle, pid })
        }

        pub fn is_running(&self) -> bool {
            // SAFETY: valid handle owned by self.
            unsafe { WaitForSingleObject(self.handle, 0) == WAIT_TIMEOUT }
        }

        pub fn exit_code(&self) -> Option<u32> {
            let mut code = 0u32;
            // SAFETY: valid handle; `code` is a valid out-pointer.
            let ok = unsafe { GetExitCodeProcess(self.handle, &mut code) };
            (ok != 0 && !self.is_running()).then_some(code)
        }

        pub fn terminate(&self) {
            // SAFETY: valid handle owned by self.
            unsafe {
                TerminateProcess(self.handle, 1);
            }
        }

        /// When the process was started.
        pub fn created(&self) -> Option<SystemTime> {
            let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
            let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
            // SAFETY: valid handle; four valid FILETIME out-pointers.
            let ok = unsafe { GetProcessTimes(self.handle, &mut created, &mut exited, &mut kernel, &mut user) };
            if ok == 0 {
                return None;
            }
            // FILETIME: 100 ns ticks since 1601-01-01.
            let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
            let unix_ticks = ticks.checked_sub(116_444_736_000_000_000)?;
            Some(UNIX_EPOCH + Duration::from_nanos(unix_ticks * 100))
        }

        pub fn image_path(&self) -> Option<PathBuf> {
            let mut buf = vec![0u16; 1024];
            let mut len = buf.len() as u32;
            // SAFETY: buffer and length describe valid writable memory.
            let ok = unsafe { QueryFullProcessImageNameW(self.handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) };
            (ok != 0).then(|| PathBuf::from(OsString::from_wide(&buf[..len as usize])))
        }
    }

    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            // SAFETY: we own the handle.
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }

    pub struct ProcInfo {
        pub pid: u32,
        pub parent: u32,
        pub path: PathBuf,
    }

    /// The file name of a running process (e.g. "Discord.exe").
    pub fn process_name(pid: u32) -> Option<String> {
        // SAFETY: as in find_processes.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE || snap.is_null() {
                return None;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snap, &mut entry) != 0;
            let mut found = None;
            while more {
                if entry.th32ProcessID == pid {
                    let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                    found = Some(OsString::from_wide(&entry.szExeFile[..len]).to_string_lossy().into_owned());
                    break;
                }
                more = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
            found
        }
    }

    /// Running processes whose file name is `exe_name` (e.g. RustDedicated.exe), with full paths.
    pub fn find_processes(exe_name: &str) -> Vec<ProcInfo> {
        let mut out = Vec::new();
        // SAFETY: snapshot handle closed below; PROCESSENTRY32W is POD with dwSize set.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE || snap.is_null() {
                return out;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snap, &mut entry) != 0;
            while more {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let name = OsString::from_wide(&entry.szExeFile[..len]);
                if name.to_string_lossy().eq_ignore_ascii_case(exe_name) {
                    let path = ProcessHandle::open(entry.th32ProcessID).and_then(|p| p.image_path()).unwrap_or_default();
                    out.push(ProcInfo { pid: entry.th32ProcessID, parent: entry.th32ParentProcessID, path });
                }
                more = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
        }
        out
    }
}

/// Run a script from `dir` in a terminal window (first one found), or in the background.
#[cfg(not(windows))]
pub fn launch_console(dir: &Path, script: &str, env: &[(&str, &str)]) -> std::io::Result<Launched> {
    let path = dir.join(script);
    let terminals = [("x-terminal-emulator", "-e"), ("gnome-terminal", "--"), ("konsole", "-e"), ("xfce4-terminal", "-x"), ("xterm", "-e")];
    for (terminal, flag) in terminals {
        let mut cmd = Command::new(terminal);
        cmd.arg(flag).arg(&path).current_dir(dir).envs(env.iter().copied());
        if let Ok(child) = cmd.spawn() {
            return Ok(Launched { shell_pid: child.id(), job: ProcessJob });
        }
    }
    let log = std::fs::File::create(dir.join("droplaunch_console.log"))?;
    let child = Command::new(&path).current_dir(dir).envs(env.iter().copied()).stdout(log.try_clone()?).stderr(log).spawn()?;
    Ok(Launched { shell_pid: child.id(), job: ProcessJob })
}

#[cfg(not(windows))]
pub use unix::{find_processes, process_name, Launched, ProcInfo, ProcessHandle, ProcessJob};

#[cfg(not(windows))]
mod unix {
    use std::path::PathBuf;
    use std::time::SystemTime;

    /// No job objects here: the server is followed by its pid instead.
    pub struct ProcessJob;

    impl ProcessJob {
        pub fn active_processes(&self) -> u32 {
            0
        }
        pub fn terminate(&self) {}
    }

    pub struct Launched {
        pub shell_pid: u32,
        pub job: ProcessJob,
    }

    pub struct ProcessHandle {
        pub pid: u32,
    }

    impl ProcessHandle {
        pub fn open(pid: u32) -> Option<Self> {
            std::path::Path::new(&format!("/proc/{pid}")).exists().then_some(Self { pid })
        }
        pub fn is_running(&self) -> bool {
            std::path::Path::new(&format!("/proc/{}", self.pid)).exists()
        }
        pub fn exit_code(&self) -> Option<u32> {
            None
        }
        pub fn terminate(&self) {
            let _ = std::process::Command::new("kill").arg("-9").arg(self.pid.to_string()).status();
        }
        pub fn created(&self) -> Option<SystemTime> {
            std::fs::metadata(format!("/proc/{}", self.pid)).and_then(|m| m.modified()).ok()
        }
    }

    pub struct ProcInfo {
        pub pid: u32,
        pub parent: u32,
        pub path: PathBuf,
    }

    pub fn process_name(pid: u32) -> Option<String> {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string())
    }

    pub fn find_processes(exe_name: &str) -> Vec<ProcInfo> {
        let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
        entries
            .flatten()
            .filter_map(|e| {
                let pid: u32 = e.file_name().to_str()?.parse().ok()?;
                let path = std::fs::read_link(e.path().join("exe")).ok()?;
                (path.file_name()?.to_str()? == exe_name).then_some(ProcInfo { pid, parent: 0, path })
            })
            .collect()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    /// A drive's root and the user's busy folders are found (any case, with or without a slash);
    /// a folder inside them is fine.
    #[test]
    fn busy_places_are_told_apart_from_a_folder_of_its_own() {
        let known = [("desktop", PathBuf::from(r"C:\Users\Map Maker\OneDrive\Desktop")), ("home", PathBuf::from(r"C:\Users\Map Maker"))];
        assert_eq!(super::busy_place(Path::new(r"D:\")), Some("drive"));
        assert_eq!(super::busy_place_among(Path::new(r"c:\users\map maker\onedrive\desktop\"), &known), Some("desktop"));
        assert_eq!(super::busy_place_among(Path::new("C:/Users/Map Maker"), &known), Some("home"));
        assert_eq!(super::busy_place_among(Path::new(r"C:\Users\Map Maker\OneDrive\Desktop\DropLaunch"), &known), None);
    }

    /// Windows tells where the user's own folders are.
    #[cfg(windows)]
    #[test]
    fn the_users_folders_are_known() {
        let kinds: Vec<_> = super::user_folders().into_iter().map(|(kind, _)| kind).collect();
        assert_eq!(kinds, ["desktop", "downloads", "documents", "home"]);
    }

    /// Ports open on this PC show up in Windows' table (opened on this PC only: no firewall question).
    #[test]
    fn open_ports_are_found_without_opening_them() {
        let tcp = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let udp = std::net::UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let (tcp_port, udp_port) = (tcp.local_addr().unwrap().port(), udp.local_addr().unwrap().port());
        assert_eq!(super::port_in_use(false, tcp_port), Some(true));
        assert_eq!(super::port_in_use(true, udp_port), Some(true));
        drop(tcp);
        drop(udp);
        assert_eq!(super::port_in_use(false, tcp_port), Some(false));
        assert_eq!(super::port_in_use(true, udp_port), Some(false));
    }

    /// Opens a console window for a moment: `cargo test -- --ignored launch_console`.
    #[test]
    #[ignore]
    fn launch_console_runs_the_script_inside_its_job() {
        let dir = std::env::temp_dir().join(format!("droplaunch-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.bat"), "@echo off\r\necho %DL_TEST%> out.txt\r\nping -n 3 127.0.0.1 >nul\r\n").unwrap();
        let launched = super::launch_console(&dir, "test.bat", &[("DL_TEST", "hello")]).unwrap();
        assert!(launched.shell_pid > 0);
        // cmd and the ping it starts are both in the job.
        let start = Instant::now();
        let mut most = 0;
        while start.elapsed() < Duration::from_secs(10) {
            let n = launched.job.active_processes();
            most = most.max(n);
            if most >= 2 && n == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(most >= 2, "only {most} process(es) seen in the job");
        assert_eq!(launched.job.active_processes(), 0, "the script didn't finish");
        assert_eq!(std::fs::read_to_string(dir.join("out.txt")).unwrap().trim(), "hello");
        let _ = std::fs::remove_dir_all(dir);
    }
}
