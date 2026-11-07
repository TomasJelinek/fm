use crate::Limits;
use std::io;

fn limit(resource: libc::__rlimit_resource_t, value: u64) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: valid exact-sized limits, applied to this worker only before parsing any content.
    if unsafe { libc::setrlimit(resource, &limit) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn restrict(limits: Limits) -> io::Result<()> {
    limit(libc::RLIMIT_AS, limits.memory_bytes)?;
    limit(libc::RLIMIT_CPU, u64::from(limits.cpu_seconds))?;
    limit(libc::RLIMIT_CORE, 0)?;
    limit(libc::RLIMIT_FSIZE, 0)?;
    limit(libc::RLIMIT_NOFILE, 3)?;
    // Drop every accidental inherited descriptor beyond the broker pipes, before denying opens.
    // SAFETY: close_range affects only this process's descriptor table, keeping stdin/out/err.
    if unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // One decoder thread per worker; host process concurrency supplies bounded CPU parallelism.
    // No open/socket/connect/exec/clone/io_uring/ptrace/pidfd/process_vm syscalls are permitted.
    let allowed = [
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_close,
        libc::SYS_fstat,
        libc::SYS_lseek,
        libc::SYS_brk,
        libc::SYS_mmap,
        libc::SYS_mprotect,
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_madvise,
        libc::SYS_futex,
        libc::SYS_clock_gettime,
        libc::SYS_clock_nanosleep,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sigaltstack,
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_getrandom,
        libc::SYS_sched_yield,
        libc::SYS_sched_getaffinity,
        libc::SYS_getcpu,
        libc::SYS_restart_syscall,
    ];
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc00000b7;
    const LOAD: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
    const EQUAL: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
    const RETURN: u16 = (libc::BPF_RET | libc::BPF_K) as u16;
    let instruction = |code, jt, jf, k| libc::sock_filter { code, jt, jf, k };
    let mut filters = vec![
        instruction(LOAD, 0, 0, 4), // seccomp_data.arch
        instruction(EQUAL, 1, 0, ARCH),
        instruction(RETURN, 0, 0, libc::SECCOMP_RET_KILL_PROCESS),
        instruction(LOAD, 0, 0, 0), // seccomp_data.nr
    ];
    for syscall in allowed {
        filters.push(instruction(EQUAL, 0, 1, syscall as u32));
        filters.push(instruction(RETURN, 0, 0, libc::SECCOMP_RET_ALLOW));
    }
    filters.push(instruction(
        RETURN,
        0,
        0,
        libc::SECCOMP_RET_ERRNO | libc::EPERM as u32,
    ));
    let program = libc::sock_fprog {
        len: filters.len() as u16,
        filter: filters.as_mut_ptr(),
    };
    // SAFETY: the BPF program is bounded and lives through the synchronous copy into the kernel.
    // no_new_privs makes the restriction irreversible. TSYNC prevents an unfiltered sibling.
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            || libc::syscall(
                libc::SYS_seccomp,
                libc::SECCOMP_SET_MODE_FILTER,
                libc::SECCOMP_FILTER_FLAG_TSYNC,
                &program as *const libc::sock_fprog,
            ) != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
