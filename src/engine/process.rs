use super::{Error, Result, fail};
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{fs::OpenOptionsExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct Control {
    pub cancelled: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
}
impl Control {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);
    }
    pub fn pause(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }
    pub fn checkpoint(&self) -> Result<()> {
        while self.paused.load(Ordering::SeqCst) && !self.cancelled.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(50));
        }
        if self.cancelled.load(Ordering::SeqCst) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

pub struct Spec {
    pub program: String,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub writable: Vec<PathBuf>,
    pub input: Option<File>,
    pub output: Option<File>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: u64,
}
impl Spec {
    pub fn new(program: &str, cwd: &Path) -> Self {
        Self {
            program: program.into(),
            args: vec![],
            cwd: cwd.into(),
            writable: vec![cwd.into()],
            input: None,
            output: None,
            stdin: None,
            timeout: 0,
        }
    }
    pub fn arg(&mut self, arg: impl Into<OsString>) -> &mut Self {
        self.args.push(arg.into());
        self
    }
    pub fn args(&mut self, args: &[&str]) -> &mut Self {
        self.args.extend(args.iter().map(OsString::from));
        self
    }
    pub fn source(&mut self, input: &File) -> Result<&mut Self> {
        self.input = Some(input.try_clone()?);
        Ok(self)
    }
}
pub struct Outcome {
    pub status: ExitStatus,
    pub stderr: String,
}

/// Host recovery tools must use host libraries, not a bundled AppImage's GTK stack.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    if std::env::var_os("ODR_APPIMAGE").is_some() {
        for variable in [
            "LD_LIBRARY_PATH",
            "GTK_EXE_PREFIX",
            "GTK_DATA_PREFIX",
            "GIO_MODULE_DIR",
            "GIO_EXTRA_MODULES",
            "GDK_PIXBUF_MODULE_FILE",
            "GSETTINGS_SCHEMA_DIR",
        ] {
            command.env_remove(variable);
        }
    }
    command
}

// Landlock denies writes everywhere except approved job directories. Read access stays
// available for executables, libraries and source input; inherited source FDs are O_RDONLY.
#[repr(C)]
struct Ruleset {
    handled_access_fs: u64,
}
#[repr(C, packed)]
struct PathRule {
    allowed_access: u64,
    parent_fd: i32,
}
struct Sandbox {
    directories: Vec<File>,
    rights: u64,
}
impl Sandbox {
    fn new(paths: &[PathBuf]) -> Result<Self> {
        let abi = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<Ruleset>(),
                0,
                1,
            )
        };
        if abi < 3 {
            return fail(
                "This kernel does not provide Landlock ABI 3 write isolation (Linux 6.2 or newer). Recovery backends will not run without source protection.",
            );
        }
        let mut rights = (1 << 1)
            | (1 << 4)
            | (1 << 5)
            | (1 << 6)
            | (1 << 7)
            | (1 << 8)
            | (1 << 9)
            | (1 << 10)
            | (1 << 11)
            | (1 << 12);
        if abi >= 2 {
            rights |= 1 << 13;
        }
        if abi >= 3 {
            rights |= 1 << 14;
        }
        let directories = paths
            .iter()
            .map(|path| {
                OpenOptions::new()
                    .read(true)
                    .custom_flags(
                        libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                    .open(path)
            })
            .collect::<std::io::Result<_>>()?;
        Ok(Self {
            directories,
            rights,
        })
    }
    fn apply(&self) -> std::io::Result<()> {
        let attrs = Ruleset {
            handled_access_fs: self.rights,
        };
        let fd = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attrs,
                std::mem::size_of::<Ruleset>(),
                0,
            )
        } as i32;
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let ruleset = unsafe { File::from_raw_fd(fd) };
        for directory in &self.directories {
            let rule = PathRule {
                allowed_access: self.rights,
                parent_fd: directory.as_raw_fd(),
            };
            if unsafe {
                libc::syscall(
                    libc::SYS_landlock_add_rule,
                    ruleset.as_raw_fd(),
                    1,
                    &rule,
                    0,
                )
            } < 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Used only on the dedicated restoration worker, after opening its two files.
/// Landlock leaves those descriptors usable but denies all new writable opens.
pub(super) fn confine_restore_thread() -> Result<()> {
    Sandbox::new(&[])?.apply()?;
    Ok(())
}

pub fn run(
    mut spec: Spec,
    control: &Control,
    mut line: impl FnMut(&str) -> Result<()>,
) -> Result<Outcome> {
    control.checkpoint()?;
    let sandbox = Sandbox::new(&spec.writable)?;
    // Record effective arguments for reproducibility; append because some adapters
    // enumerate before extracting. Paths are arguments, never shell commands.
    let mut audit = OpenOptions::new()
        .create(true)
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(spec.cwd.join("commands.jsonl"))?;
    if audit.metadata()?.len() < 4 * 1024 * 1024 {
        writeln!(
            audit,
            "{}",
            serde_json::json!({"program":spec.program,"arguments":spec.args.iter().map(|a|a.to_string_lossy()).collect::<Vec<_>>() })
        )?;
    }
    let input = spec.input.take();
    let mut command = command(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env("LC_ALL", "C")
        .env("TMPDIR", &spec.cwd)
        .env("XDG_CACHE_HOME", &spec.cwd)
        .stderr(Stdio::piped())
        .process_group(0);
    command.stdin(if spec.stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(if let Some(output) = spec.output {
        Stdio::from(output)
    } else {
        Stdio::piped()
    });
    // Only async-signal-safe syscalls are used after fork.
    unsafe {
        command.pre_exec(move || {
            sandbox.apply()?;
            if let Some(input) = &input {
                if libc::dup2(input.as_raw_fd(), 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|error| {
        Error::Message(format!(
            "Cannot start {} with source protection: {error}",
            spec.program
        ))
    })?;
    let pid = child.id() as i32;
    // Wake as soon as a short extraction exits, while keeping the existing
    // 50 ms pause/cancel supervision interval. The child is reaped below, so
    // its PID cannot be recycled before the supervisor finishes signalling it.
    let exit_fd = unsafe {
        let fd = libc::syscall(libc::SYS_pidfd_open, pid, 0) as i32;
        (fd >= 0).then(|| OwnedFd::from_raw_fd(fd))
    };
    if let Some(script) = spec.stdin {
        if script.len() > 65536 {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.wait();
            return fail("Backend input script exceeds its size limit.");
        }
        if let Some(mut stdin) = child.stdin.take() {
            thread::spawn(move || {
                let _ = stdin.write_all(&script);
            });
        }
    }
    let stderr = child.stderr.take().unwrap();
    let stderr_thread = thread::spawn(move || {
        let mut stderr = stderr;
        let mut tail = Vec::new();
        let mut buffer = [0_u8; 8192];
        while let Ok(count) = stderr.read(&mut buffer) {
            if count == 0 {
                break;
            }
            tail.extend_from_slice(&buffer[..count]);
            if tail.len() > 65536 {
                tail.drain(..tail.len() - 65536);
            }
        }
        String::from_utf8_lossy(&tail).into_owned()
    });
    let stdout = child.stdout.take();
    let (tx, rx) = mpsc::channel();
    let job_control = control.clone();
    thread::spawn(move || {
        let began = Instant::now();
        let mut paused = false;
        let mut terminated = None;
        let mut low_space = false;
        let mut space_check = Instant::now() - Duration::from_secs(2);
        loop {
            if space_check.elapsed() >= Duration::from_secs(1) {
                low_space =
                    super::safety::free_bytes(&spec.cwd).is_ok_and(|free| free < 8 * 1024 * 1024);
                space_check = Instant::now();
            }
            let cancel = job_control.cancelled.load(Ordering::SeqCst)
                || (spec.timeout > 0 && began.elapsed().as_secs() >= spec.timeout)
                || low_space;
            if cancel && terminated.is_none() {
                unsafe {
                    libc::kill(-pid, libc::SIGCONT);
                    libc::kill(-pid, libc::SIGTERM);
                }
                terminated = Some(Instant::now());
            }
            if terminated.is_some_and(|time| time.elapsed() > Duration::from_secs(2)) {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
            let requested = job_control.paused.load(Ordering::SeqCst) && !cancel;
            if paused != requested {
                unsafe {
                    libc::kill(
                        -pid,
                        if requested {
                            libc::SIGSTOP
                        } else {
                            libc::SIGCONT
                        },
                    );
                }
                paused = requested;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    let _ = tx.send(Ok((status, cancel, low_space)));
                    break;
                }
                Ok(None) => {
                    if let Some(fd) = &exit_fd {
                        let mut event = libc::pollfd {
                            fd: fd.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        };
                        if unsafe { libc::poll(&mut event, 1, 50) } < 0 {
                            thread::sleep(Duration::from_millis(50));
                        }
                    } else {
                        thread::sleep(Duration::from_millis(50));
                    }
                }
                Err(error) => {
                    unsafe {
                        libc::kill(-pid, libc::SIGCONT);
                        libc::kill(-pid, libc::SIGKILL);
                    }
                    let _ = child.wait();
                    let _ = tx.send(Err(error));
                    break;
                }
            }
        }
    });
    let mut parse_error = None;
    if let Some(stdout) = stdout {
        let mut reader = BufReader::new(stdout);
        loop {
            match bounded_line(&mut reader) {
                Ok(Some(value)) => {
                    if let Err(error) = line(&value) {
                        parse_error = Some(error);
                        unsafe {
                            libc::kill(-pid, libc::SIGCONT);
                            libc::kill(-pid, libc::SIGKILL);
                        }
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    parse_error = Some(error);
                    unsafe {
                        libc::kill(-pid, libc::SIGKILL);
                    }
                    break;
                }
            }
        }
    }
    let (status, terminated, low_space) = rx
        .recv()
        .map_err(|_| Error::Message("Backend supervisor stopped unexpectedly.".into()))??;
    let stderr = stderr_thread
        .join()
        .unwrap_or_else(|_| "Cannot read backend error output.".into());
    if let Some(error) = parse_error {
        return Err(error);
    }
    if control.cancelled.load(Ordering::SeqCst) {
        return Err(Error::Cancelled);
    }
    if terminated {
        if low_space {
            return fail(format!(
                "{} stopped because workspace free space fell below the 8 MiB reserve. Partial artifacts were retained.",
                spec.program
            ));
        }
        return fail(format!(
            "{} reached the configured timeout. Partial artifacts were retained.",
            spec.program
        ));
    }
    Ok(Outcome { status, stderr })
}

fn bounded_line(reader: &mut impl BufRead) -> Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok((!line.is_empty()).then(|| String::from_utf8_lossy(&line).into_owned()));
        }
        let end = buffer.iter().position(|&byte| byte == b'\n');
        let count = end.map(|i| i + 1).unwrap_or(buffer.len());
        if line.len() + count > 1024 * 1024 {
            return fail("Backend output contains a line larger than 1 MiB.");
        }
        line.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(Some(
                String::from_utf8_lossy(&line)
                    .trim_end_matches(['\r', '\n'])
                    .to_owned(),
            ));
        }
    }
}

pub fn probe(program: &str, args: &[&str]) -> Option<String> {
    let mut child = command(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let stderr = child.stderr.take()?;
    let read = thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.take(32768).read_to_string(&mut text);
        text
    });
    let err = thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.take(32768).read_to_string(&mut text);
        text
    });
    let began = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(_) => break,
            None if began.elapsed() < Duration::from_secs(3) => {
                thread::sleep(Duration::from_millis(20))
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let text = format!(
        "{}{}",
        read.join().unwrap_or_default(),
        err.join().unwrap_or_default()
    );
    let version = text
        .lines()
        .find(|line| {
            let lower = line.trim().to_ascii_lowercase();
            !lower.starts_with('-')
                && (lower.starts_with("version:")
                    || line.split_whitespace().any(|word| {
                        word.trim_start_matches(['v', 'V'])
                            .bytes()
                            .next()
                            .is_some_and(|b| b.is_ascii_digit())
                            && word.contains('.')
                            && !word.contains('/')
                            && !word.starts_with('(')
                    }))
        })
        .unwrap_or("version not reported")
        .trim();
    // A binary digest disambiguates patched distribution builds and tools whose
    // help banner omits a version. It is recorded with the attempt, never inferred later.
    use sha2::{Digest, Sha256};
    let digest = std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(program))
                .find(|p| p.is_file())
        })
        .and_then(|path| {
            let mut file = File::open(path).ok()?;
            let mut hash = Sha256::new();
            let mut buffer = [0_u8; 65536];
            loop {
                let count = file.read(&mut buffer).ok()?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            Some(format!("{:x}", hash.finalize()))
        });
    Some(format!(
        "{}{}",
        version.chars().take(160).collect::<String>(),
        digest
            .map(|hash| format!(" [sha256:{hash}]"))
            .unwrap_or_default()
    ))
}
