//! Private process entrypoint. No filesystem request is accepted before isolation succeeds.
use std::ffi::OsString;

#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "Linux jail setup requires libc in the private, single-threaded helper"
)]
pub fn run(arguments: &[OsString]) -> std::io::Result<()> {
    use panel_fs::worker::process::{read_request, write_response};
    use panel_fs::{ConfinedFs, FsWorkerHandler, FsWorkerRequest, SymlinkPolicy};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    if arguments.len() != 4 {
        return Err(std::io::Error::other("Invalid worker arguments"));
    }
    let id = |value: &OsString| -> std::io::Result<u32> {
        value
            .to_str()
            .and_then(|text| text.parse().ok())
            .filter(|id| (10000..=59999).contains(id))
            .ok_or_else(|| std::io::Error::other("Invalid tenant identity"))
    };
    let uid = id(&arguments[0])?;
    let gid = id(&arguments[1])?;
    let policy: SymlinkPolicy = serde_json::from_str(
        arguments[3]
            .to_str()
            .ok_or_else(|| std::io::Error::other("Invalid policy"))?,
    )
    .map_err(std::io::Error::other)?;
    // SAFETY: These calls affect only this freshly exec'd, single-threaded helper process.
    if unsafe { libc::geteuid() } != 0 {
        return Err(std::io::Error::other("Worker setup requires root"));
    }
    close_inherited_descriptors()?;
    let anchor = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&arguments[2])?;
    let metadata = anchor.metadata()?;
    if metadata.uid() != uid || metadata.gid() != gid {
        return Err(std::io::Error::other("Tenant home ownership mismatch"));
    }
    // Pin the directory inode before chroot, so pathname replacements cannot change the jail.
    // SAFETY: Valid owned fd and NUL-terminated constant paths; errors fail closed.
    unsafe {
        check(libc::fchdir(anchor.as_raw_fd()))?;
        check(libc::chroot(c".".as_ptr()))?;
        check(libc::chdir(c"/".as_ptr()))?;
    }
    drop(anchor);
    set_limit(libc::RLIMIT_CORE, 0)?;
    set_limit(libc::RLIMIT_NOFILE, 256)?;
    set_limit(libc::RLIMIT_CPU, 60)?;
    set_limit(libc::RLIMIT_AS, 768 * 1024 * 1024)?;
    // SAFETY: Clear all groups/capability retention and all saved IDs in this helper only.
    unsafe {
        check(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))?;
        check(libc::prctl(libc::PR_SET_KEEPCAPS, 0, 0, 0, 0))?;
        check(libc::setgroups(0, std::ptr::null()))?;
        check(libc::setresgid(gid, gid, gid))?;
        check(libc::setresuid(uid, uid, uid))?;
        check(libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0))?;
        libc::umask(0o022);
        if libc::getuid() != uid
            || libc::geteuid() != uid
            || libc::getgid() != gid
            || libc::getegid() != gid
            || libc::getgroups(0, std::ptr::null_mut()) != 0
        {
            return Err(std::io::Error::other("Worker privilege drop failed"));
        }
    }
    verify_no_capabilities()?;
    let fs = ConfinedFs::new("/", policy).map_err(std::io::Error::other)?;
    let handler = FsWorkerHandler::new(fs);
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Some(request) = read_request(&mut input)? {
        let shutdown = matches!(request, FsWorkerRequest::Shutdown);
        write_response(&mut output, &handler.handle(request))?;
        if shutdown {
            break;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "Query this helper's capability masks through the Linux ABI"
)]
fn verify_no_capabilities() -> std::io::Result<()> {
    // Linux capability ABI v3: header(version,pid), two data words with
    // effective/permitted/inheritable masks. Refuse unusual inherited securebits
    // configurations instead of assuming setresuid cleared every capability.
    let mut header = [0x2008_0522_u32, 0];
    let mut capabilities = [[0_u32; 3]; 2];
    // SAFETY: Correctly sized/aligned ABI buffers; pid=0 queries this helper only.
    if unsafe {
        libc::syscall(
            libc::SYS_capget,
            header.as_mut_ptr(),
            capabilities.as_mut_ptr(),
        )
    } == -1
    {
        return Err(std::io::Error::last_os_error());
    }
    if capabilities.iter().flatten().any(|mask| *mask != 0) {
        return Err(std::io::Error::other("Worker retained capabilities"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn check(result: libc::c_int) -> std::io::Result<()> {
    if result == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "Apply resource limits to the private helper through libc"
)]
fn set_limit(resource: libc::__rlimit_resource_t, value: libc::rlim_t) -> std::io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: Valid pointer; this is a private helper, never the running agent process.
    check(unsafe { libc::setrlimit(resource, &limit) })
}

#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "Close inherited raw descriptors before accepting tenant requests"
)]
fn close_inherited_descriptors() -> std::io::Result<()> {
    // Enumerate before entering the jail. Dropping ReadDir closes its own descriptor too.
    let descriptors: Vec<i32> = std::fs::read_dir("/proc/self/fd")?
        .map(|entry| {
            entry.and_then(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<i32>().ok())
                    .ok_or_else(|| std::io::Error::other("Invalid descriptor"))
            })
        })
        .collect::<Result<_, _>>()?;
    for fd in descriptors.into_iter().filter(|fd| *fd > 2) {
        // SAFETY: No other threads exist; stdin/stdout/stderr remain the sole transport.
        unsafe {
            libc::close(fd);
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn run(_arguments: &[OsString]) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "Isolated filesystem workers require Linux",
    ))
}
