//! Child processes whose whole tree can be stopped at once.
//!
//! A coding agent started in the background can start processes of its own (a shell, a
//! language server, a search tool). Killing only the process LocalFlow started would leave
//! those running. Here the child is created suspended, placed in a new job object with
//! kill-on-close, and only then allowed to run, so not even its first instruction runs
//! outside the job. Terminating the job, or dropping this value, ends every process in it,
//! and the operating system does the same if LocalFlow itself crashes.

use std::{
    io,
    process::{Child, Command, ExitStatus},
};

/// Creation flag: give a console program a hidden console instead of opening a window.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

/// A child process, and every process it starts, held in one kill-on-close job.
pub struct ContainedChild {
    child: Child,
    #[cfg(windows)]
    job: windows_job::Job,
}

impl ContainedChild {
    /// Spawn `command` with a hidden console inside a new job. Any creation flags already set
    /// on `command` are replaced. Fails rather than running the program uncontained.
    pub fn spawn_hidden(command: &mut Command) -> io::Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;

            let job = windows_job::Job::kill_on_close()?;
            command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
            let mut child = command.spawn()?;
            if let Err(error) = job
                .assign(&child)
                .and_then(|()| windows_job::resume(&child))
            {
                // Still suspended or outside the job: never let it run.
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            Ok(Self { child, job })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                child: command.spawn()?,
            })
        }
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Process ids currently in the job: the child and everything it started that is still
    /// running. Only the child itself where jobs are not available.
    pub fn process_ids(&self) -> Vec<u32> {
        #[cfg(windows)]
        {
            self.job.process_ids()
        }
        #[cfg(not(windows))]
        {
            vec![self.child.id()]
        }
    }

    /// End the child and everything it started, then reap the child.
    pub fn kill_tree(&mut self) {
        #[cfg(windows)]
        self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ContainedChild {
    fn drop(&mut self) {
        // Anything still running in the job (a grandchild left behind by a child that has
        // exited, for example) ends here. The job handle closes after this, which on Windows
        // would also kill it.
        self.kill_tree();
    }
}

/// Whether a process with this id is still running. Used to check that a stopped tree is
/// really gone. An id can be reused by a new process later, so this is only meaningful
/// shortly after the process was seen.
pub fn is_process_running(pid: u32) -> bool {
    #[cfg(windows)]
    {
        windows_job::is_running(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

#[cfg(windows)]
mod windows_job {
    use std::{io, os::windows::io::AsRawHandle, process::Child};

    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT},
            System::{
                Diagnostics::ToolHelp::{
                    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                    THREADENTRY32,
                },
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicProcessIdList,
                    JobObjectExtendedLimitInformation, QueryInformationJobObject,
                    SetInformationJobObject, TerminateJobObject, JOBOBJECT_BASIC_PROCESS_ID_LIST,
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                },
                Threading::{
                    OpenProcess, OpenThread, ResumeThread, WaitForSingleObject,
                    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, THREAD_SUSPEND_RESUME,
                },
            },
        },
    };

    /// An owned job handle. Closing it kills every process still in the job.
    pub struct Job(HANDLE);

    // The handle is owned by this value alone and only closed in `Drop`.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn kill_on_close() -> io::Result<Self> {
            unsafe {
                let handle = CreateJobObjectW(None, PCWSTR::null()).map_err(to_io)?;
                let job = Self(handle);
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .map_err(to_io)?;
                Ok(job)
            }
        }

        pub fn assign(&self, child: &Child) -> io::Result<()> {
            unsafe {
                AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle())).map_err(to_io)
            }
        }

        pub fn terminate(&self) {
            unsafe {
                let _ = TerminateJobObject(self.0, 1);
            }
        }

        pub fn process_ids(&self) -> Vec<u32> {
            // The list struct ends in a one-element array; give it room for many more.
            const CAPACITY: usize = 256;
            let header = std::mem::size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
            let bytes = header + CAPACITY * std::mem::size_of::<usize>();
            let mut buffer = vec![0usize; bytes.div_ceil(std::mem::size_of::<usize>())];
            unsafe {
                if QueryInformationJobObject(
                    Some(self.0),
                    JobObjectBasicProcessIdList,
                    buffer.as_mut_ptr() as *mut core::ffi::c_void,
                    (buffer.len() * std::mem::size_of::<usize>()) as u32,
                    None,
                )
                .is_err()
                {
                    return Vec::new();
                }
                let list = &*(buffer.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST);
                let count = (list.NumberOfProcessIdsInList as usize).min(CAPACITY);
                let ids = std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count);
                ids.iter().map(|&id| id as u32).collect()
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Let a process created suspended run. Such a process has exactly one thread, which is
    /// found in a thread snapshot because the standard library closes its handle.
    pub fn resume(child: &Child) -> io::Result<()> {
        let pid = child.id();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).map_err(to_io)?;
            let mut entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut resumed = false;
            let mut more = Thread32First(snapshot, &mut entry).is_ok();
            while more {
                if entry.th32OwnerProcessID == pid {
                    if let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID)
                    {
                        resumed |= ResumeThread(thread) != u32::MAX;
                        let _ = CloseHandle(thread);
                    }
                }
                more = Thread32Next(snapshot, &mut entry).is_ok();
            }
            let _ = CloseHandle(snapshot);
            if resumed {
                Ok(())
            } else {
                Err(io::Error::other("could not resume the new process"))
            }
        }
    }

    pub fn is_running(pid: u32) -> bool {
        unsafe {
            match OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            ) {
                Ok(process) => {
                    let still_running = WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
                    let _ = CloseHandle(process);
                    still_running
                }
                Err(_) => false,
            }
        }
    }

    fn to_io(error: windows::core::Error) -> io::Error {
        io::Error::other(error.message())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };

    /// `cmd` running a second `cmd` that waits for input on the inherited pipe: a two-level
    /// tree that stays up until it is killed, with no window, no timer and no network.
    fn waiting_tree() -> ContainedChild {
        let mut command = Command::new("cmd.exe");
        command
            .args(["/d", "/c", "cmd.exe", "/d", "/k"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        ContainedChild::spawn_hidden(&mut command).expect("spawn cmd in a job")
    }

    fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn the_child_runs_and_its_own_children_join_the_job() {
        let tree = waiting_tree();
        assert!(
            wait_for(|| tree.process_ids().len() >= 2),
            "the inner cmd should start inside the job: {:?}",
            tree.process_ids()
        );
        assert!(tree.process_ids().contains(&tree.id()));
    }

    #[test]
    fn killing_the_tree_ends_every_process_in_it() {
        let mut tree = waiting_tree();
        assert!(wait_for(|| tree.process_ids().len() >= 2));
        let ids = tree.process_ids();
        assert!(ids.iter().all(|&id| is_process_running(id)));

        tree.kill_tree();

        assert!(
            wait_for(|| ids.iter().all(|&id| !is_process_running(id))),
            "processes left running: {ids:?}"
        );
    }

    #[test]
    fn dropping_the_handle_ends_every_process_in_it() {
        let tree = waiting_tree();
        assert!(wait_for(|| tree.process_ids().len() >= 2));
        let ids = tree.process_ids();

        drop(tree);

        assert!(
            wait_for(|| ids.iter().all(|&id| !is_process_running(id))),
            "processes left running: {ids:?}"
        );
    }

    #[test]
    fn a_program_that_does_not_exist_is_an_error_not_a_panic() {
        let mut command = Command::new(r"C:\definitely\not\here\nothing.exe");
        assert!(ContainedChild::spawn_hidden(&mut command).is_err());
    }
}
