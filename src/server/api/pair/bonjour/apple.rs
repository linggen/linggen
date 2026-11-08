//! Registration through the OS's own responder (mDNSResponder, DNS-SD API).
//!
//! The registration is a connection to mDNSResponder: it stays on the LAN
//! while the connection is open and goes away with the process. One thread
//! holds it and waits on its socket; if mDNSResponder restarts, the socket
//! closes and the thread registers again.

use std::ffi::{c_char, c_void, CStr, CString};
use std::time::Duration;

/// Wait before registering again after a failure or a dropped connection.
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// A TXT entry is one length byte, then at most this many bytes.
const TXT_ENTRY_MAX: usize = 255;
/// `kDNSServiceFlagsAdd`: the reply reports the record as registered.
const FLAG_ADD: u32 = 0x2;

type ServiceRef = *mut c_void;
type RegisterReply =
    extern "C" fn(ServiceRef, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);

// libSystem (libsystem_dnssd) — see <dns_sd.h>.
extern "C" {
    fn DNSServiceRegister(
        sd_ref: *mut ServiceRef,
        flags: u32,
        interface_index: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        host: *const c_char,
        port_be: u16,
        txt_len: u16,
        txt_record: *const c_void,
        callback: RegisterReply,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRefSockFD(sd_ref: ServiceRef) -> i32;
    fn DNSServiceProcessResult(sd_ref: ServiceRef) -> i32;
    fn DNSServiceRefDeallocate(sd_ref: ServiceRef);
}

pub(super) fn advertise(port: u16) {
    let spawned = std::thread::Builder::new()
        .name("bonjour".into())
        .spawn(move || serve(port));
    if let Err(e) = spawned {
        tracing::warn!("[bonjour] advertise not started: {e}");
    }
}

/// Runs for the daemon's lifetime: register, hold, register again if dropped.
fn serve(port: u16) {
    loop {
        match Registration::new(port) {
            Ok(reg) => {
                reg.hold();
                tracing::warn!("[bonjour] mDNSResponder dropped the record; registering again");
            }
            Err(code) => tracing::warn!("[bonjour] advertise failed: DNS-SD error {code}"),
        }
        std::thread::sleep(RETRY_AFTER);
    }
}

/// One open registration; dropping it withdraws the record.
struct Registration(ServiceRef);

impl Registration {
    fn new(port: u16) -> Result<Self, i32> {
        let record = super::record();
        // A NUL inside a name can't be sent; fall back rather than fail.
        let name = CString::new(record.name).unwrap_or_else(|_| c"Mac".into());
        let regtype = CString::new(super::SERVICE_TYPE).expect("static type");
        let txt = txt_bytes(&record.txt);
        let mut sd_ref: ServiceRef = std::ptr::null_mut();
        // Host NULL: the SRV target is the Mac's own `.local` name, which
        // mDNSResponder owns — no second claimant, so no computer rename.
        let err = unsafe {
            DNSServiceRegister(
                &mut sd_ref,
                0,
                0, // every interface mDNSResponder serves
                name.as_ptr(),
                regtype.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                port.to_be(),
                txt.len() as u16,
                txt.as_ptr().cast(),
                on_reply,
                usize::from(port) as *mut c_void,
            )
        };
        if err != 0 {
            return Err(err);
        }
        Ok(Self(sd_ref))
    }

    /// Block while the connection lives, handing each reply to `on_reply`.
    fn hold(&self) {
        let fd = unsafe { DNSServiceRefSockFD(self.0) };
        if fd < 0 {
            return;
        }
        loop {
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let n = unsafe { libc::poll(&mut pfd, 1, -1) };
            if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            if n < 0 || pfd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                return;
            }
            if unsafe { DNSServiceProcessResult(self.0) } != 0 {
                return;
            }
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        unsafe { DNSServiceRefDeallocate(self.0) };
    }
}

/// mDNSResponder's answer: registered (with the name it settled on, which
/// differs from ours after a conflict), or an error.
extern "C" fn on_reply(
    _sd_ref: ServiceRef,
    flags: u32,
    err: i32,
    name: *const c_char,
    _regtype: *const c_char,
    _domain: *const c_char,
    context: *mut c_void,
) {
    if err != 0 {
        tracing::warn!("[bonjour] registration error: DNS-SD error {err}");
        return;
    }
    if flags & FLAG_ADD == 0 {
        return;
    }
    let name = if name.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    };
    let port = context as usize;
    tracing::info!("[bonjour] advertising _linggen._tcp as '{name}' on port {port}");
}

/// TXT record wire form: each `key=value` behind its length byte, cut at a
/// character boundary to fit the 255-byte entry limit.
fn txt_bytes(pairs: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (key, value) in pairs {
        let entry = fit(format!("{key}={value}"));
        out.push(entry.len() as u8);
        out.extend_from_slice(entry.as_bytes());
    }
    out
}

fn fit(mut entry: String) -> String {
    if entry.len() <= TXT_ENTRY_MAX {
        return entry;
    }
    let mut cut = TXT_ENTRY_MAX;
    while !entry.is_char_boundary(cut) {
        cut -= 1;
    }
    entry.truncate(cut);
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txt_entries_are_length_prefixed() {
        let txt = txt_bytes(&[("name", "Mac".into()), ("account", String::new())]);
        assert_eq!(txt, b"\x08name=Mac\x08account=".to_vec());
    }

    #[test]
    fn a_long_entry_is_cut_on_a_char_boundary() {
        let txt = txt_bytes(&[("name", "灵".repeat(100))]);
        let len = usize::from(txt[0]);
        assert!(len <= TXT_ENTRY_MAX);
        assert_eq!(txt.len(), len + 1);
        assert!(std::str::from_utf8(&txt[1..]).is_ok());
    }
}
