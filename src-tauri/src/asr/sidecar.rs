//! Process helpers for local ASR sidecars: hidden launch, guaranteed cleanup, and memory
//! measurement. Platform-specific code stays inside this module.

use std::{path::Path, process::Child, process::Command};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A `Command` for a console sidecar that never flashes a terminal window on Windows.
pub fn hidden_sidecar_command(program: &Path) -> Command {
    let mut command = Command::new(program);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
}

/// Ties a child process's lifetime to this value. On Windows the child is placed in a job
/// object with kill-on-close, so the operating system terminates it even when LocalFlow
/// crashes or is killed and no destructor runs. Elsewhere this is a no-op and the owner's
/// `Drop` is the only cleanup.
pub struct ChildLifetimeGuard {
    #[cfg(windows)]
    job: isize,
}

// The raw job handle is only ever closed once, from `Drop`.
unsafe impl Send for ChildLifetimeGuard {}
unsafe impl Sync for ChildLifetimeGuard {}

impl ChildLifetimeGuard {
    /// Bind `child` to a new guard. Returns `None` when the platform call fails; the caller
    /// still owns the child and must kill it on drop.
    #[cfg(windows)]
    pub fn attach(child: &Child) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::{CloseHandle, HANDLE},
                System::JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                },
            },
        };

        unsafe {
            let job = CreateJobObjectW(None, PCWSTR::null()).ok()?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            let configured = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .is_ok();
            let assigned =
                configured && AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())).is_ok();

            if !assigned {
                let _ = CloseHandle(job);
                return None;
            }

            Some(Self {
                job: job.0 as isize,
            })
        }
    }

    #[cfg(not(windows))]
    pub fn attach(_child: &Child) -> Option<Self> {
        None
    }
}

impl Drop for ChildLifetimeGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::Foundation::{CloseHandle, HANDLE};
            let _ = CloseHandle(HANDLE(self.job as *mut core::ffi::c_void));
        }
    }
}

/// Peak resident memory of `child` in bytes, if the platform can report it. Works for a
/// process that has already exited as long as the `Child` has not been dropped.
#[cfg(windows)]
pub fn peak_memory_bytes(child: &Child) -> Option<u64> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
    };

    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let ok = unsafe {
        K32GetProcessMemoryInfo(HANDLE(child.as_raw_handle()), &mut counters, size).as_bool()
    };

    ok.then_some(counters.PeakWorkingSetSize as u64)
}

#[cfg(not(windows))]
pub fn peak_memory_bytes(_child: &Child) -> Option<u64> {
    None
}

/// Peak resident memory of the current process in bytes, for engines that run in-process.
#[cfg(windows)]
pub fn current_process_peak_memory_bytes() -> Option<u64> {
    use windows::Win32::System::{
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };

    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size).as_bool() };

    ok.then_some(counters.PeakWorkingSetSize as u64)
}

#[cfg(not(windows))]
pub fn current_process_peak_memory_bytes() -> Option<u64> {
    None
}

/// Process id that owns the TCP listener on `port` (IPv4, any local address), if one
/// exists. Used to confirm that the process answering on a sidecar's port is the sidecar
/// LocalFlow started and not another local program that grabbed the port first.
#[cfg(windows)]
pub fn tcp_listener_pid(port: u16) -> Option<u32> {
    use windows::Win32::{
        NetworkManagement::IpHelper::{
            GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
        },
        Networking::WinSock::AF_INET,
    };

    // The table can grow between the sizing call and the read, so retry a few times.
    for _ in 0..4 {
        let mut size = 0u32;
        unsafe {
            GetExtendedTcpTable(
                None,
                &mut size,
                false,
                AF_INET.0 as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            );
        }
        if size == 0 {
            return None;
        }

        // u64 storage keeps the buffer aligned for the row structs.
        let mut buffer = vec![0u64; (size as usize).div_ceil(8) + 64];
        let mut capacity = (buffer.len() * 8) as u32;
        let status = unsafe {
            GetExtendedTcpTable(
                Some(buffer.as_mut_ptr().cast()),
                &mut capacity,
                false,
                AF_INET.0 as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if status != 0 {
            continue;
        }

        let base = buffer.as_ptr().cast::<u8>();
        let count = unsafe { *base.cast::<u32>() } as usize;
        let rows = unsafe {
            base.add(std::mem::size_of::<u32>())
                .cast::<MIB_TCPROW_OWNER_PID>()
        };
        let available = (buffer.len() * 8 - std::mem::size_of::<u32>())
            / std::mem::size_of::<MIB_TCPROW_OWNER_PID>();

        return (0..count.min(available))
            .map(|index| unsafe { rows.add(index).read_unaligned() })
            // The port is stored in network byte order in the low 16 bits.
            .find(|row| u16::from_be((row.dwLocalPort & 0xffff) as u16) == port)
            .map(|row| row.dwOwningPid);
    }

    None
}

#[cfg(not(windows))]
pub fn tcp_listener_pid(_port: u16) -> Option<u32> {
    None
}

/// Upper bound on whisper.cpp threads. On a Core Ultra 7 165H (6 performance cores) six
/// threads and eight measured the same for tiny.en and base.en, and published guidance is
/// that going past the performance cores of a hybrid CPU hurts, so the cap is six: the same
/// speed with more cores left free (see `docs/benchmarks/ASR_BENCHMARK.md`).
pub const MAX_ASR_THREADS: usize = 6;

/// Threads to give whisper.cpp: the logical CPU count minus one (left for capture and the
/// UI), kept between two and [`MAX_ASR_THREADS`].
pub fn default_asr_thread_count() -> usize {
    std::thread::available_parallelism()
        .map(|available| available.get().saturating_sub(1))
        .unwrap_or(4)
        .clamp(2, MAX_ASR_THREADS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_count_stays_within_bounds() {
        let threads = default_asr_thread_count();
        assert!((2..=MAX_ASR_THREADS).contains(&threads));
    }

    #[cfg(windows)]
    #[test]
    fn listener_lookup_finds_this_process_and_ignores_free_ports() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        assert_eq!(tcp_listener_pid(port), Some(std::process::id()));

        drop(listener);
        assert_eq!(tcp_listener_pid(port), None);
    }

    #[cfg(windows)]
    #[test]
    fn guard_kills_the_child_when_dropped() {
        use std::process::Stdio;

        // `ping -n 30` runs for about thirty seconds unless it is killed.
        let mut child = hidden_sidecar_command(Path::new("ping"))
            .args(["-n", "30", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("ping should be available on Windows");

        let guard = ChildLifetimeGuard::attach(&child).expect("job object should attach");
        assert!(child.try_wait().unwrap().is_none());
        assert!(peak_memory_bytes(&child).is_some_and(|bytes| bytes > 0));

        drop(guard);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child survived its lifetime guard"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
