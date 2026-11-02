//! GNU dd copies only through the descriptors already validated by the wizard.
use super::{Control, Error, Result, fail, process};
use std::{
    fs::File,
    io::{Read, Seek},
    os::unix::process::CommandExt,
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

pub(super) fn check_available() -> Result<()> {
    if process::probe("dd", &["--version"]).is_some_and(|version| {
        version.starts_with("dd (coreutils) ") || version.starts_with("dd (GNU coreutils) ")
    }) {
        Ok(())
    } else {
        fail("Image restoration requires GNU dd. Install GNU coreutils and try again.")
    }
}

pub(super) fn command() -> Command {
    let mut command = process::command("dd");
    command.env("LC_ALL", "C").process_group(0);
    let parent = std::process::id() as libc::pid_t;
    // Only async-signal-safe operations after fork. Do not leave a destructive
    // child running if the ODR worker exits, including during fork/exec setup.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
    command
}

pub(super) fn copy(
    source: &mut File,
    target: &mut File,
    size: u64,
    control: &Control,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    control.checkpoint()?;
    if size == 0 || source.metadata()?.len() != size {
        return fail("The source image length changed before restoration.");
    }
    source.rewind()?;
    target.rewind()?;
    progress(0);
    control.checkpoint()?;
    let mut command = command();
    command
        .args([
            "bs=1M",
            "iflag=fullblock",
            "conv=notrunc,fsync",
            "status=progress",
        ])
        .arg(format!("count={size}B"))
        // No if=/of= paths, shell, or new device open. dup2 preserves the
        // source's read-only access and the target's exclusive synchronous open.
        .stdin(Stdio::from(source.try_clone()?))
        .stdout(Stdio::from(target.try_clone()?));
    let result = run(&mut command, target, control, &mut progress);
    // dd has exited and been reaped, even on cancellation or supervision errors.
    // Flush here as well: a killed child cannot be relied upon to run its fsync.
    target.sync_all()?;
    result?;
    // GNU dd can exit successfully at an early EOF. Its inherited descriptors
    // share offsets with ours; check both instead of trusting exit status alone.
    if source.stream_position()? != size || target.stream_position()? != size {
        return fail("GNU dd did not copy the complete image; restoration is incomplete.");
    }
    progress(size);
    control.checkpoint()
}

struct RunningChild(Child);
impl Drop for RunningChild {
    fn drop(&mut self) {
        // Also covers an unexpected return or panic in the progress callback.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn run(
    command: &mut Command,
    target: &mut File,
    control: &Control,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    let mut child = RunningChild(command.stderr(Stdio::piped()).spawn().map_err(|error| {
        Error::Message(format!(
            "Cannot start GNU dd for image restoration: {error}"
        ))
    })?);
    let stderr = child.0.stderr.take().unwrap();
    let reader = thread::spawn(move || read_stderr(stderr));
    let status = supervise(&mut child.0, target, control, &mut progress);
    drop(child); // Stop/reap before joining the reader, including on I/O errors.
    let diagnostic = reader
        .join()
        .map_err(|_| Error::Message("Cannot read GNU dd error output.".into()))??;
    let status = status?;
    control.checkpoint()?;
    if !status.success() {
        return fail(format!("GNU dd failed ({status}): {}", diagnostic.trim()));
    }
    Ok(())
}

fn supervise(
    child: &mut Child,
    target: &mut File,
    control: &Control,
    mut progress: impl FnMut(u64),
) -> Result<ExitStatus> {
    let mut interrupted = None;
    let mut paused = false;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        // The child has not been reaped, so its PID cannot be reused here.
        let pid = child.id() as libc::pid_t;
        let cancelled = control.cancelled.load(Ordering::SeqCst);
        if cancelled && interrupted.is_none() {
            unsafe {
                libc::kill(pid, libc::SIGCONT);
                libc::kill(pid, libc::SIGINT);
            }
            interrupted = Some(Instant::now());
        }
        if interrupted.is_some_and(|start| start.elapsed() >= Duration::from_secs(2)) {
            child.kill()?;
        }
        let requested = control.paused.load(Ordering::SeqCst) && !cancelled;
        if paused != requested {
            unsafe {
                libc::kill(
                    pid,
                    if requested {
                        libc::SIGSTOP
                    } else {
                        libc::SIGCONT
                    },
                );
            }
            paused = requested;
        }
        // Query the shared descriptor offset without moving it. Progress does
        // not depend on parsing dd's carriage-return-delimited diagnostic text.
        progress(target.stream_position()?);
        thread::sleep(Duration::from_millis(50));
    }
}

fn read_stderr(mut stderr: impl Read) -> std::io::Result<String> {
    let mut tail = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = match stderr.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        tail.extend_from_slice(&buffer[..count]);
        if tail.len() > 65536 {
            tail.drain(..tail.len() - 65536);
        }
    }
    Ok(String::from_utf8_lossy(&tail).into_owned())
}
