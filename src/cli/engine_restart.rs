//! After `ling update` / `ling update --rollback` swaps the binary, restart
//! the engine daemons that were running from it, so the new version is the
//! one serving. The same rule as install.sh's `restart_engine_daemon`
//! (linggensite): an engine is a `ling --web …` process; one started from the
//! swapped path is stopped (TERM, then KILL) and started again with its own
//! arguments and working directory — no browser. An engine at any other path
//! (Linggen.app carries its own `ling`) is left alone and named.
//!
//! The running engines are found BEFORE the swap — afterwards the old
//! process's executable resolves to `ling.prev`. If a restarted engine gives
//! no `/api/health` in time, the binary is swapped back to `ling.prev` and
//! the engines started again on that.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// One process seen running: how it was started.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    /// The executable's path as the kernel knows it (None when unreadable).
    pub exe: Option<PathBuf>,
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// The engines found before a swap.
#[derive(Debug, Default)]
pub struct Found {
    /// Started from the binary being replaced: restart these.
    pub ours: Vec<ProcessInfo>,
    /// Started from somewhere else: left alone.
    pub others: Vec<ProcessInfo>,
}

pub struct Opts {
    /// Where `/api/health` is asked when the engine's argv names none.
    pub default_host: String,
    pub default_port: u16,
    /// The restarted engine's stdout/stderr (bare `ling` uses ~/.linggen/ling.log).
    pub log: PathBuf,
    pub stop_timeout: Duration,
    pub health_timeout: Duration,
    /// For the `--version` probe of a rollback.
    pub probe_timeout: Duration,
}

impl Opts {
    pub fn new(default_host: String, default_port: u16) -> Self {
        Self {
            default_host,
            default_port,
            log: crate::paths::linggen_home().join("ling.log"),
            stop_timeout: Duration::from_secs(10),
            health_timeout: Duration::from_secs(60),
            probe_timeout: Duration::from_secs(10),
        }
    }
}

// ── Finding ──────────────────────────────────────────────────────────────

/// The `ling --web` engines on this machine, split by whether they were
/// started from `target`. Call before the swap.
pub fn find(target: &Path, binary_name: &str) -> Found {
    let me = std::process::id();
    let mut pids = pgrep_web();
    if let Some(pid) = pid_file_pid() {
        pids.push(pid);
    }
    pids.sort_unstable();
    pids.dedup();
    let mut found = Found::default();
    for pid in pids.into_iter().filter(|p| *p != me) {
        let Some(info) = process_info(pid) else {
            continue;
        };
        match classify(target, binary_name, &info) {
            Some(true) => found.ours.push(info),
            Some(false) => found.others.push(info),
            None => {}
        }
    }
    found
}

/// None: not a `ling --web` engine. Some(true): started from `target`.
pub fn classify(target: &Path, binary_name: &str, info: &ProcessInfo) -> Option<bool> {
    if !info.argv.iter().skip(1).any(|a| a == "--web") {
        return None;
    }
    let argv0 = info.argv.first().map(PathBuf::from);
    let named = |p: &Path| p.file_name().is_some_and(|n| n == binary_name);
    let is_ling = info.exe.as_deref().is_some_and(named) || argv0.as_deref().is_some_and(named);
    if !is_ling {
        return None;
    }
    let target = canonical(target);
    let same = |p: &Path| p.is_absolute() && canonical(p) == target;
    Some(info.exe.as_deref().is_some_and(same) || argv0.as_deref().is_some_and(same))
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn pgrep_web() -> Vec<u32> {
    let Ok(out) = std::process::Command::new("pgrep")
        .args(["-f", "--", "--web"])
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

fn pid_file_pid() -> Option<u32> {
    std::fs::read_to_string(crate::cli::daemon::agent_pid_file())
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[cfg(target_os = "macos")]
pub fn process_info(pid: u32) -> Option<ProcessInfo> {
    Some(ProcessInfo {
        pid,
        exe: mac::exe(pid),
        argv: mac::argv(pid)?,
        cwd: mac::cwd(pid),
    })
}

#[cfg(target_os = "linux")]
pub fn process_info(pid: u32) -> Option<ProcessInfo> {
    let proc = PathBuf::from(format!("/proc/{pid}"));
    let raw = std::fs::read(proc.join("cmdline")).ok()?;
    let argv = raw
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    Some(ProcessInfo {
        pid,
        exe: std::fs::read_link(proc.join("exe")).ok(),
        argv,
        cwd: std::fs::read_link(proc.join("cwd")).ok(),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn process_info(_pid: u32) -> Option<ProcessInfo> {
    None
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    pub fn exe(pid: u32) -> Option<PathBuf> {
        let mut buf = vec![0u8; 4 * 1024];
        // SAFETY: the buffer outlives the call and its size is passed.
        let n =
            unsafe { libc::proc_pidpath(pid as i32, buf.as_mut_ptr().cast(), buf.len() as u32) };
        (n > 0).then(|| PathBuf::from(OsStr::from_bytes(&buf[..n as usize])))
    }

    /// KERN_PROCARGS2: argc, the exec path, NULs, then argv.
    pub fn argv(pid: u32) -> Option<Vec<String>> {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
        let mut size: libc::size_t = 0;
        // SAFETY: a size query (null buffer) on a 3-entry mib.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 || size < 4 {
            return None;
        }
        let mut buf = vec![0u8; size];
        // SAFETY: the buffer holds `size` bytes, which the kernel may lower.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                buf.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 || size < 4 {
            return None;
        }
        buf.truncate(size);
        let argc = i32::from_ne_bytes(buf[..4].try_into().ok()?).max(0) as usize;
        let rest = &buf[4..];
        // Skip the exec path and the NUL padding after it.
        let mut i = rest.iter().position(|b| *b == 0)?;
        while i < rest.len() && rest[i] == 0 {
            i += 1;
        }
        let argv: Vec<String> = rest[i..]
            .split(|b| *b == 0)
            .take(argc)
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        (argv.len() == argc).then_some(argv)
    }

    pub fn cwd(pid: u32) -> Option<PathBuf> {
        // SAFETY: proc_vnodepathinfo is plain old data; zeroed is valid.
        let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
        // SAFETY: `info` is `size` bytes and outlives the call.
        let n = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&mut info as *mut libc::proc_vnodepathinfo).cast(),
                size,
            )
        };
        if n != size {
            return None;
        }
        // SAFETY: vip_path is a NUL-terminated C string of MAXPATHLEN bytes.
        let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
        let bytes = path.to_bytes();
        (!bytes.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(bytes)))
    }
}

// ── Restarting ───────────────────────────────────────────────────────────

/// Say which engines were left alone because they run another binary.
pub fn report_others(found: &Found) {
    for e in &found.others {
        let at = e
            .exe
            .as_deref()
            .map(|p| p.display().to_string())
            .or_else(|| e.argv.first().cloned())
            .unwrap_or_default();
        println!(
            "  Left alone: the engine at {at} (PID {}) — not the binary this update replaced.",
            e.pid
        );
    }
}

/// Restart `engines` on `<dir>/<binary_name>` (just swapped in). If one
/// gives no health in time, swap `ling.prev` back and restart them on it.
pub async fn restart(
    dir: &Path,
    binary_name: &str,
    engines: &[ProcessInfo],
    opts: &Opts,
) -> Result<()> {
    if engines.is_empty() {
        return Ok(());
    }
    let target = dir.join(binary_name);
    for e in engines {
        println!(
            "[{binary_name}] Restarting the running engine (PID {}, port {}) on the swapped-in binary...",
            e.pid,
            port_of(e, opts)
        );
    }
    let failure = match relaunch(&target, engines, opts).await {
        Ok(pids) => {
            report_up(binary_name, engines, &pids, opts);
            return Ok(());
        }
        Err(f) => f,
    };

    println!(
        "[{binary_name}] {}; swapping back to {binary_name}.prev",
        failure.why
    );
    stop_all(&failure.started, opts).await;
    let (from, to) = crate::cli::self_update::rollback_binary(dir, binary_name, opts.probe_timeout)
        .map_err(|e| anyhow::anyhow!("{} and the swap back failed: {e:#}", failure.why))?;
    let restarted = failure.stopped_old;
    match relaunch(&target, &engines[..restarted.min(engines.len())], opts).await {
        Ok(pids) => {
            report_up(binary_name, engines, &pids, opts);
            anyhow::bail!(
                "[{binary_name}] {} — rolled back v{from} -> v{to}; the engine runs v{to}",
                failure.why
            )
        }
        Err(again) => anyhow::bail!(
            "[{binary_name}] {} — rolled back v{from} -> v{to}, but that engine did not start either: {}",
            failure.why,
            again.why
        ),
    }
}

fn report_up(binary_name: &str, engines: &[ProcessInfo], pids: &[u32], opts: &Opts) {
    for (e, pid) in engines.iter().zip(pids) {
        println!(
            "[{binary_name}] Engine is back on port {} (PID {} -> {pid}).",
            port_of(e, opts),
            e.pid
        );
    }
}

struct Failure {
    why: String,
    /// Engines this attempt started (to stop before retrying).
    started: Vec<u32>,
    /// How many of the old engines were stopped (those need starting again).
    stopped_old: usize,
}

/// Stop each engine, start it again from `target`, and wait for health.
/// Ok(new pids) in engine order.
async fn relaunch(
    target: &Path,
    engines: &[ProcessInfo],
    opts: &Opts,
) -> std::result::Result<Vec<u32>, Failure> {
    let mut started = Vec::new();
    for (i, e) in engines.iter().enumerate() {
        stop(e.pid, opts.stop_timeout).await;
        let fail = |why: String, started: Vec<u32>| Failure {
            why,
            started,
            stopped_old: i + 1,
        };
        let pid = match spawn(target, e, opts) {
            Ok(pid) => pid,
            Err(err) => {
                return Err(fail(
                    format!("could not start the engine: {err:#}"),
                    started,
                ))
            }
        };
        started.push(pid);
        let (host, port) = (health_host(e, opts), port_of(e, opts));
        if !wait_health(pid, &host, port, opts.health_timeout).await {
            let why = format!(
                "the restarted engine gave no /api/health on port {port} within {}s",
                opts.health_timeout.as_secs()
            );
            return Err(fail(why, started));
        }
    }
    Ok(started)
}

fn spawn(target: &Path, e: &ProcessInfo, opts: &Opts) -> Result<u32> {
    if let Some(parent) = opts.log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let log = std::fs::File::create(&opts.log)?;
    let cwd = e
        .cwd
        .clone()
        .filter(|d| d.is_dir())
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"));
    let mut cmd = std::process::Command::new(target);
    cmd.args(e.argv.iter().skip(1))
        .current_dir(cwd)
        .stdout(log.try_clone()?)
        .stderr(log)
        .stdin(std::process::Stdio::null());
    #[cfg(unix)]
    cmd.process_group(0);
    Ok(cmd.spawn()?.id())
}

async fn stop_all(pids: &[u32], opts: &Opts) {
    for pid in pids {
        stop(*pid, opts.stop_timeout).await;
    }
}

/// TERM, wait up to `timeout`, then KILL.
async fn stop(pid: u32, timeout: Duration) {
    signal(pid, libc::SIGTERM);
    let deadline = Instant::now() + timeout;
    while alive(pid) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if alive(pid) {
        signal(pid, libc::SIGKILL);
        for _ in 0..30 {
            if !alive(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn signal(pid: u32, sig: i32) {
    // SAFETY: kill(2) with a positive pid only signals that process.
    unsafe {
        libc::kill(pid as i32, sig);
    }
}

/// Running (a child of ours that exited is reaped, not counted).
fn alive(pid: u32) -> bool {
    // SAFETY: WNOHANG never blocks; a pid that isn't our child is ECHILD.
    unsafe {
        libc::waitpid(pid as i32, std::ptr::null_mut(), libc::WNOHANG);
        libc::kill(pid as i32, 0) == 0
    }
}

async fn wait_health(pid: u32, host: &str, port: u16, timeout: Duration) -> bool {
    let url = format!("http://{host}:{port}/api/health");
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
    else {
        return false;
    };
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(r) = client.get(&url).send().await {
            if r.status().is_success() {
                return true;
            }
        }
        if !alive(pid) {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

fn flag(argv: &[String], name: &str) -> Option<String> {
    let eq = format!("{name}=");
    argv.iter().enumerate().find_map(|(i, a)| {
        if a == name {
            argv.get(i + 1).cloned()
        } else {
            a.strip_prefix(&eq).map(str::to_string)
        }
    })
}

fn port_of(e: &ProcessInfo, opts: &Opts) -> u16 {
    flag(&e.argv, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(opts.default_port)
}

fn health_host(e: &ProcessInfo, opts: &Opts) -> String {
    let host = flag(&e.argv, "--host").unwrap_or_else(|| opts.default_host.clone());
    match host.as_str() {
        "" | "0.0.0.0" | "::" | "[::]" | "localhost" => "127.0.0.1".into(),
        h if h.contains(':') && !h.starts_with('[') => format!("[{h}]"),
        h => h.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(exe: Option<&str>, argv: &[&str]) -> ProcessInfo {
        ProcessInfo {
            pid: 1,
            exe: exe.map(PathBuf::from),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            cwd: None,
        }
    }

    #[test]
    fn only_a_web_engine_from_the_swapped_path_is_ours() {
        let t = Path::new("/opt/x/bin/ling");
        let c = |i: &ProcessInfo| classify(t, "ling", i);
        // Bare `ling` spawns `<abs exe> --web --port N`.
        assert_eq!(
            c(&info(
                Some("/opt/x/bin/ling"),
                &["/opt/x/bin/ling", "--web", "--port", "9527"]
            )),
            Some(true)
        );
        // Started through PATH: argv0 is bare, the exe tells.
        assert_eq!(
            c(&info(Some("/opt/x/bin/ling"), &["ling", "--web"])),
            Some(true)
        );
        // Linggen.app's own engine.
        let app = "/Applications/Linggen.app/Contents/MacOS/ling";
        assert_eq!(
            c(&info(
                Some(app),
                &[app, "--web", "--idle-shutdown-secs", "600"]
            )),
            Some(false)
        );
        // Not engines: no --web, or not ling.
        assert_eq!(
            c(&info(
                Some("/opt/x/bin/ling"),
                &["/opt/x/bin/ling", "update"]
            )),
            None
        );
        assert_eq!(c(&info(Some("/usr/bin/vim"), &["vim", "--web"])), None);
    }

    #[test]
    fn port_and_health_host_come_from_the_engines_own_flags() {
        let o = Opts::new("127.0.0.1".into(), 9527);
        let e = info(None, &["ling", "--web", "--port=9600", "--host", "0.0.0.0"]);
        assert_eq!(
            (port_of(&e, &o), health_host(&e, &o)),
            (9600, "127.0.0.1".into())
        );
        let e = info(None, &["ling", "--web"]);
        assert_eq!(
            (port_of(&e, &o), health_host(&e, &o)),
            (9527, "127.0.0.1".into())
        );
        let e = info(None, &["ling", "--web", "--host", "192.168.1.5"]);
        assert_eq!(health_host(&e, &o), "192.168.1.5");
    }

    #[test]
    fn a_process_reads_back_with_its_argv_and_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 30; :", "x", "--web"])
            .current_dir(dir.path())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let got = process_info(child.id());
        let _ = child.kill();
        let _ = child.wait();
        let got = got.expect("process info");
        assert_eq!(got.argv, ["/bin/sh", "-c", "sleep 30; :", "x", "--web"]);
        assert_eq!(canonical(&got.cwd.unwrap()), canonical(dir.path()));
        // /bin/sh may be another shell underneath (bash on macOS).
        assert!(got.exe.unwrap().is_absolute());
    }

    // ── restart on temp dirs, with a fake `ling` serving /api/health ────

    /// A fake `ling`: `--version` says `version`; `--web --port N` runs
    /// `web` (with $3 = the port) after leaving its pid in ./engine.pid.
    fn fake(path: &Path, version: &str, web: &str) {
        let body = format!(
            "#!/bin/sh\ncase \"$1\" in\n--version) echo 'ling {version}' ;;\n--web) echo $$ > engine.pid; {web} ;;\nesac\n"
        );
        std::fs::write(path, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Serves its working directory, where api/health is a file.
    const SERVES: &str = "exec python3 -m http.server \"$3\" --bind 127.0.0.1";

    fn have_python() -> bool {
        std::process::Command::new("python3")
            .args(["-c", "import http.server"])
            .status()
            .is_ok_and(|s| s.success())
    }

    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    struct Rig {
        bin: tempfile::TempDir,
        work: tempfile::TempDir,
        port: u16,
        opts: Opts,
    }

    impl Rig {
        fn new() -> Self {
            let (bin, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            std::fs::create_dir_all(work.path().join("api")).unwrap();
            std::fs::write(work.path().join("api/health"), "ok").unwrap();
            let mut opts = Opts::new("127.0.0.1".into(), 1);
            opts.log = work.path().join("ling.log");
            opts.health_timeout = Duration::from_secs(15);
            opts.stop_timeout = Duration::from_secs(3);
            Rig {
                bin,
                work,
                port: free_port(),
                opts,
            }
        }

        fn target(&self) -> PathBuf {
            self.bin.path().join("ling")
        }

        /// The engine as bare `ling` leaves it: `<target> --web --port N`
        /// running in the work dir.
        async fn running(&self) -> ProcessInfo {
            let argv: Vec<String> = [
                self.target().display().to_string(),
                "--web".into(),
                "--port".into(),
                self.port.to_string(),
            ]
            .into();
            let child = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .current_dir(self.work.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            assert!(wait_health(child.id(), "127.0.0.1", self.port, Duration::from_secs(15)).await);
            ProcessInfo {
                pid: child.id(),
                exe: None,
                argv,
                cwd: Some(self.work.path().to_path_buf()),
            }
        }

        fn engine_pid(&self) -> u32 {
            std::fs::read_to_string(self.work.path().join("engine.pid"))
                .unwrap()
                .trim()
                .parse()
                .unwrap()
        }

        fn version(&self) -> String {
            crate::cli::self_update::probe_version(&self.target(), "ling", Duration::from_secs(5))
                .unwrap()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            if let Ok(pid) = std::fs::read_to_string(self.work.path().join("engine.pid")) {
                if let Ok(pid) = pid.trim().parse::<u32>() {
                    signal(pid, libc::SIGKILL);
                }
            }
        }
    }

    #[tokio::test]
    async fn a_running_engine_is_restarted_on_the_new_binary() {
        if !have_python() {
            return;
        }
        let rig = Rig::new();
        fake(&rig.target(), "1.0.0", SERVES);
        let old = rig.running().await;
        // The update swaps in 2.0.0 and keeps 1.0.0 as ling.prev.
        std::fs::rename(rig.target(), rig.bin.path().join("ling.prev")).unwrap();
        fake(&rig.target(), "2.0.0", SERVES);

        restart(rig.bin.path(), "ling", &[old.clone()], &rig.opts)
            .await
            .unwrap();

        assert!(!alive(old.pid), "the old engine still runs");
        let new = rig.engine_pid();
        assert_ne!(new, old.pid);
        assert!(wait_health(new, "127.0.0.1", rig.port, Duration::from_secs(2)).await);
        let argv = process_info(new).unwrap().argv;
        assert!(argv.iter().any(|a| a == &rig.port.to_string()), "{argv:?}");
        assert_eq!(rig.version(), "2.0.0");
    }

    #[tokio::test]
    async fn an_engine_that_will_not_start_rolls_back_and_restarts_the_previous() {
        if !have_python() {
            return;
        }
        let rig = Rig::new();
        fake(&rig.target(), "1.0.0", SERVES);
        let old = rig.running().await;
        std::fs::rename(rig.target(), rig.bin.path().join("ling.prev")).unwrap();
        fake(&rig.target(), "2.0.0", "exit 1");

        let err = restart(rig.bin.path(), "ling", &[old.clone()], &rig.opts)
            .await
            .unwrap_err();

        assert!(
            format!("{err:#}").contains("rolled back v2.0.0 -> v1.0.0"),
            "{err:#}"
        );
        assert_eq!(rig.version(), "1.0.0");
        let back = rig.engine_pid();
        assert_ne!(back, old.pid);
        assert!(wait_health(back, "127.0.0.1", rig.port, Duration::from_secs(2)).await);
    }

    #[tokio::test]
    async fn nothing_running_means_nothing_restarted() {
        let rig = Rig::new();
        fake(&rig.target(), "2.0.0", "exit 1");
        restart(rig.bin.path(), "ling", &[], &rig.opts)
            .await
            .unwrap();
        assert_eq!(rig.version(), "2.0.0");
    }
}
