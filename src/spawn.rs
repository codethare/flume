// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spawn a program detached from flume's session, process group, and
//! controlling terminal (setsid + double fork), matching rill-ed spawn.zig
//! and the behavior of kwm/dwl/etc.

use std::ffi::CString;

pub fn spawn_detached(argv: &[String]) -> Result<(), String> {
    if argv.is_empty() {
        return Err("empty argv".into());
    }
    let argv_c: Vec<CString> = argv
        .iter()
        .map(|a| CString::new(a.as_str()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("nul byte in argv: {e}"))?;
    let envp: Vec<CString> = std::env::vars_os()
        .filter_map(|(k, v)| {
            let mut s = k.into_string().ok()?;
            s.push('=');
            s.push_str(&v.into_string().ok()?);
            CString::new(s).ok()
        })
        .collect();

    // SAFETY: raw fork/exec per rill-ed spawn.zig. The parent returns
    // immediately; children only call async-signal-safe libc functions
    // (setsid, sigprocmask, fork, execve, exit).
    unsafe {
        let pid = libc::fork();
        if pid < 0 {
            return Err(format!("fork failed: {}", std::io::Error::last_os_error()));
        }
        if pid > 0 {
            return Ok(()); // parent returns immediately
        }

        // First child: new session, fully detached from the controlling
        // terminal and process group.
        if libc::setsid() < 0 {
            libc::exit(1);
        }

        // Reset the signal mask so the spawned program inherits a clean mask.
        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut mask);
        libc::sigprocmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());

        // Second fork prevents the grandchild from ever becoming a session
        // leader and reacquiring a controlling terminal.
        let pid2 = libc::fork();
        if pid2 < 0 {
            libc::exit(1);
        }
        if pid2 > 0 {
            libc::exit(0);
        }

        // Grandchild: exec the target program (never returns on success).
        if exec_search(&argv_c, &envp).is_err() {
            // The parent has already returned, so a failed exec is otherwise
            // invisible (rill-ed prints here too).
            child_warn("flume: failed to run ");
            child_warn(&argv[0]);
            child_warn("\n");
        }
        libc::exit(1);
    }
}

/// Write a diagnostic from the forked child to stderr. Only async-signal-safe
/// calls are legal after fork, so this is a plain write(2) of caller-provided
/// (already allocated) bytes; the caller must not allocate either.
fn child_warn(message: &str) {
    child_warn_to(2, message);
}

fn child_warn_to(fd: i32, message: &str) {
    // SAFETY: writing already-allocated bytes to the given fd.
    unsafe {
        libc::write(fd, message.as_ptr().cast(), message.len());
    }
}

/// exec with PATH search (rill-ed execveSearch). Only called in the forked
/// grandchild; never returns on success.
fn exec_search(argv: &[CString], envp: &[CString]) -> Result<(), ()> {
    let file = argv[0].to_str().map_err(|_| ())?;
    let mut argv_p: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
    argv_p.push(std::ptr::null());
    let mut envp_p: Vec<*const libc::c_char> = envp.iter().map(|e| e.as_ptr()).collect();
    envp_p.push(std::ptr::null());

    if let Some(pos) = file.find('/') {
        let file_c = CString::new(file).map_err(|_| ())?;
        let _ = pos;
        // SAFETY: execve in the forked grandchild; argv/envp are null-terminated.
        unsafe {
            libc::execve(file_c.as_ptr(), argv_p.as_ptr(), envp_p.as_ptr());
        }
        return Err(());
    }

    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/bin:/usr/bin".into());
    for dir in path.split(':') {
        let full = match CString::new(format!("{dir}/{file}")) {
            Ok(full) => full,
            Err(_) => continue,
        };
        // SAFETY: as above.
        unsafe {
            libc::execve(full.as_ptr(), argv_p.as_ptr(), envp_p.as_ptr());
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EACCES) | Some(libc::EPERM) | Some(libc::ENOENT) | Some(libc::ENOTDIR) => {
                continue;
            }
            _ => return Err(()),
        }
    }
    Err(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_argv_is_rejected() {
        assert!(super::spawn_detached(&[]).is_err());
    }

    #[test]
    fn nul_byte_in_argv_is_rejected() {
        let argv = vec!["alacritty".to_string(), "a\0b".to_string()];
        assert!(super::spawn_detached(&argv).is_err());
    }

    /// The child's diagnostic path is a raw write(2): check it through a pipe.
    #[test]
    fn child_warn_writes_to_the_given_fd() {
        let mut fds = [0i32; 2];
        // SAFETY: plain pipe(2) with a valid array.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        super::child_warn_to(fds[1], "flume: x");
        let mut buf = [0u8; 8];
        // SAFETY: reading from the pipe we just wrote to.
        let n = unsafe { libc::read(fds[0], buf.as_mut_ptr().cast(), buf.len()) };
        assert_eq!(n, 8);
        assert_eq!(&buf, b"flume: x");
        // SAFETY: closing both ends.
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }
}
