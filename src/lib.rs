use core::fmt;
use derive_more::From;
use std::{
    ffi::{OsStr, OsString},
    path::Path,
    sync::Arc,
};

use anyhow::{bail, Context};
use log::log;
use openssh::{KnownHosts::Strict, Stdio};
use openssh_sftp_client::{fs::Fs, Sftp};
use recipes::apt::Apt;
use tokio::io::{AsyncRead, AsyncReadExt};
use type_map::concurrent::TypeMap;

pub mod local;
pub mod recipes;

struct Arg {
    kind: ArgKind,
    display_placeholder: Option<String>,
}

impl ArgKind {
    pub fn escaped(value: impl AsRef<str>) -> Self {
        Self::Escaped(value.as_ref().into())
    }

    pub fn raw(value: impl AsRef<OsStr>) -> Self {
        Self::Raw(value.as_ref().into())
    }
}

impl Arg {
    pub fn escaped(value: impl AsRef<str>) -> Self {
        Arg {
            kind: ArgKind::escaped(value),
            display_placeholder: None,
        }
    }

    pub fn raw(value: impl AsRef<OsStr>) -> Self {
        Arg {
            kind: ArgKind::raw(value),
            display_placeholder: None,
        }
    }
}

#[derive(From)]
enum ArgKind {
    Escaped(String),
    Raw(OsString),
}

impl fmt::Debug for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(placeholder) = &self.display_placeholder {
            write!(f, "{placeholder}")
        } else {
            match &self.kind {
                ArgKind::Escaped(arg) => write!(f, "{arg:?}"),
                ArgKind::Raw(arg) => write!(f, "Raw({arg:?})"),
            }
        }
    }
}

pub struct Command<'a> {
    session: &'a Session,
    command: Vec<Arg>,
    command_log_level: log::Level,
    stdout_log_level: log::Level,
    stderr_log_level: log::Level,
    allow_failure: bool,
}

impl<'a> Command<'a> {
    pub fn arg(mut self, arg: impl AsRef<str>) -> Self {
        self.command.push(Arg::escaped(arg));
        self
    }

    pub fn raw_arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.command.push(Arg::raw(arg));
        self
    }

    pub fn redacted_arg(mut self, arg: impl AsRef<str>, placeholder: impl AsRef<str>) -> Self {
        self.command.push(Arg {
            kind: ArgKind::escaped(arg),
            display_placeholder: Some(placeholder.as_ref().into()),
        });
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        self.command
            .extend(args.into_iter().map(|arg| Arg::escaped(arg)));
        self
    }

    pub fn raw_args(mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Self {
        self.command
            .extend(args.into_iter().map(|arg| Arg::raw(arg)));
        self
    }

    pub fn prepend_args(mut self, args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut new_args: Vec<_> = args.into_iter().map(|arg| Arg::escaped(arg)).collect();
        new_args.append(&mut self.command);
        self.command = new_args;
        self
    }

    pub fn user(mut self, user: Option<&str>) -> Self {
        if let Some(user) = user {
            self = self.prepend_args(["sudo", "--login", "--user", user]);
        }
        self
    }

    pub async fn exit_code(self) -> anyhow::Result<i32> {
        self.allow_failure()
            .run()
            .await
            .map(|output| output.exit_code)
    }

    pub fn hide_all_output(self) -> Self {
        self.hide_stdout().hide_stderr()
    }

    pub fn hide_stdout(mut self) -> Self {
        self.stdout_log_level = log::Level::Trace;
        self
    }

    pub fn stdout_log_level(mut self, level: log::Level) -> Self {
        self.stdout_log_level = level;
        self
    }

    pub fn hide_stderr(mut self) -> Self {
        self.stderr_log_level = log::Level::Trace;
        self
    }

    pub fn stderr_log_level(mut self, level: log::Level) -> Self {
        self.stderr_log_level = level;
        self
    }

    pub fn hide_command(mut self) -> Self {
        self.command_log_level = log::Level::Trace;
        self
    }

    pub fn command_log_level(mut self, level: log::Level) -> Self {
        self.command_log_level = level;
        self
    }

    pub fn allow_failure(mut self) -> Self {
        self.allow_failure = true;
        self
    }

    pub async fn run(self) -> anyhow::Result<CommandOutput> {
        if self.command.is_empty() {
            bail!("cannot run empty command");
        }
        log!(self.command_log_level, "running {:?}", self.command);
        let mut cmd = match &self.command[0].kind {
            ArgKind::Escaped(cmd) => self.session.inner.command(cmd),
            ArgKind::Raw(cmd) => self.session.inner.raw_command(cmd),
        };
        for arg in &self.command[1..] {
            match &arg.kind {
                ArgKind::Escaped(arg) => {
                    cmd.arg(arg);
                }
                ArgKind::Raw(arg) => {
                    cmd.raw_arg(arg);
                }
            }
        }
        cmd.stdin(Stdio::null());
        cmd.stderr(Stdio::piped());
        cmd.stdout(Stdio::piped());
        let mut child = cmd.spawn().await?;
        let stderr_reader = child.stderr().take().context("missing stderr")?;
        let stdout_reader = child.stdout().take().context("missing stdout")?;
        let stderr_task = tokio::spawn(handle_output(
            stderr_reader,
            self.stderr_log_level,
            "stderr: ",
        ));
        let stdout_task = tokio::spawn(handle_output(
            stdout_reader,
            self.stdout_log_level,
            "stdout: ",
        ));
        let status = child.wait().await?;
        let exit_code = status.code().context("missing exit code")?;
        if !self.allow_failure && exit_code != 0 {
            bail!("failed with exit code {}", exit_code);
        }
        Ok(CommandOutput {
            exit_code,
            stdout: stdout_task.await??,
            stderr: stderr_task.await??,
        })
    }
}

async fn handle_output(
    reader: impl AsyncRead,
    log_level: log::Level,
    prefix: &str,
) -> anyhow::Result<String> {
    let mut output = String::new();
    let mut vec = Vec::new();
    tokio::pin!(reader);
    loop {
        let size = reader.read_buf(&mut vec).await?;
        if size == 0 {
            break;
        }
        while let Some(index) = vec.iter().position(|i| *i == b'\n') {
            let line = std::str::from_utf8(&vec[..=index])?;
            log!(log_level, "{}{}", prefix, &line[..line.len() - 1]);
            output.push_str(line);
            vec.drain(..=index);
        }
    }
    if !vec.is_empty() {
        let line = std::str::from_utf8(&vec)?;
        log!(log_level, "{}{}[eof]", prefix, line);
        output.push_str(line);
    }
    Ok(output)
}

pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub struct Session {
    destination: String,
    inner: Arc<openssh::Session>,
    #[allow(dead_code)]
    sftp_child: openssh::Child<Arc<openssh::Session>>,
    sftp: Sftp,
    fs: Fs,
    cache: TypeMap,
}

impl Session {
    pub async fn connect(destination: impl AsRef<str>) -> anyhow::Result<Self> {
        let session = openssh::Session::connect_mux(destination.as_ref(), Strict).await?;
        let session = Arc::new(session);
        let mut sftp_child = openssh::Session::to_subsystem(session.clone(), "sftp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .await?;

        let sftp = Sftp::new(
            sftp_child.stdin().take().unwrap(),
            sftp_child.stdout().take().unwrap(),
            Default::default(),
        )
        .await?;

        Ok(Session {
            destination: destination.as_ref().into(),
            inner: session,
            sftp_child,
            fs: sftp.fs(),
            sftp,
            cache: TypeMap::new(),
        })
    }

    pub fn command<S: AsRef<str>, I: IntoIterator<Item = S>>(&self, command: I) -> Command<'_> {
        Command {
            session: self,
            command: command.into_iter().map(|s| Arg::escaped(s)).collect(),
            command_log_level: log::Level::Info,
            stdout_log_level: log::Level::Info,
            stderr_log_level: log::Level::Error,
            allow_failure: false,
        }
    }

    pub fn raw_command<S: AsRef<OsStr>, I: IntoIterator<Item = S>>(
        &self,
        command: I,
    ) -> Command<'_> {
        Command {
            session: self,
            command: command.into_iter().map(|s| Arg::raw(s)).collect(),
            command_log_level: log::Level::Info,
            stdout_log_level: log::Level::Info,
            stderr_log_level: log::Level::Error,
            allow_failure: false,
        }
    }

    pub fn sftp(&mut self) -> &mut Sftp {
        &mut self.sftp
    }

    pub fn fs(&mut self) -> &mut Fs {
        &mut self.fs
    }

    pub async fn upload(
        &mut self,
        local_paths: impl IntoIterator<Item = impl AsRef<Path>>,
        remote_parent_path: impl AsRef<Path>,
        remote_user: Option<&str>,
    ) -> anyhow::Result<()> {
        if !self
            .fs
            .metadata(remote_parent_path.as_ref())
            .await?
            .file_type()
            .context("missing file type for remote_parent_path")?
            .is_dir()
        {
            bail!(
                "upload destination {:?} is not a directory",
                remote_parent_path.as_ref()
            );
        }
        self.install_package("rsync").await?;
        let mut command = local::Command::new([
            "rsync",
            "--itemize-changes",
            "--recursive",
            "--links",
            "--perms",
            "--times",
            "--compress",
            "--delete",
        ])
        .hide_command();
        if let Some(remote_user) = remote_user {
            if remote_user
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'))
            {
                bail!("unsafe user: {remote_user:?}");
            }
            command = command
                .arg("--rsync-path")
                .arg(format!("sudo --user {remote_user} rsync"));
        }
        for arg in local_paths {
            command = command.arg(arg.as_ref().to_str().context("non-utf8 path")?);
        }
        command
            .arg(format!(
                "{}:{}",
                self.destination,
                remote_parent_path
                    .as_ref()
                    .to_str()
                    .context("non-utf8 path")?
            ))
            .run()
            .await?;

        Ok(())
    }

    pub fn cache(&mut self) -> &mut TypeMap {
        &mut self.cache
    }
}
