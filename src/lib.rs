use core::fmt;
use std::{
    ffi::{OsStr, OsString},
    path::Path,
    sync::Arc,
};

use anyhow::{bail, Context};
use log::{info, log};
use openssh::{KnownHosts::Strict, Stdio};
use openssh_sftp_client::{fs::Fs, Sftp};
use recipes::apt::install_package;
use tokio::io::{AsyncRead, AsyncReadExt};
use type_map::concurrent::TypeMap;

pub mod local;
pub mod recipes;

enum Arg {
    Escaped(String),
    Raw(OsString),
}

impl fmt::Debug for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Escaped(arg) => write!(f, "{:?}", arg),
            Self::Raw(arg) => write!(f, "Raw({:?})", arg),
        }
    }
}

pub struct Command<'a> {
    session: &'a Session,
    command: Vec<Arg>,
    stdout_log_level: log::Level,
    stderr_log_level: log::Level,
    allow_failure: bool,
}

impl<'a> Command<'a> {
    pub fn arg(mut self, arg: impl AsRef<str>) -> Self {
        self.command.push(Arg::Escaped(arg.as_ref().into()));
        self
    }

    pub fn raw_arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.command.push(Arg::Raw(arg.as_ref().into()));
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        self.command.extend(
            args.into_iter()
                .map(|arg| Arg::Escaped(arg.as_ref().into())),
        );
        self
    }

    pub fn raw_args(mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Self {
        self.command
            .extend(args.into_iter().map(|arg| Arg::Raw(arg.as_ref().into())));
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

    pub fn hide_stderr(mut self) -> Self {
        self.stderr_log_level = log::Level::Trace;
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
        info!("running {:?}", self.command);
        let mut cmd = match &self.command[0] {
            Arg::Escaped(cmd) => self.session.inner.command(cmd),
            Arg::Raw(cmd) => self.session.inner.raw_command(cmd),
        };
        for arg in &self.command[1..] {
            match arg {
                Arg::Escaped(arg) => {
                    cmd.arg(arg);
                }
                Arg::Raw(arg) => {
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
            command: command
                .into_iter()
                .map(|s| Arg::Escaped(s.as_ref().into()))
                .collect(),
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
            command: command
                .into_iter()
                .map(|s| Arg::Raw(s.as_ref().into()))
                .collect(),
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
        install_package(self, "rsync").await?;
        let mut command = local::Command::new([
            "rsync",
            //"--archive",
            "--recursive",
            "--links",
            "--perms",
            "--times",
            "--verbose",
            "--compress",
            "--human-readable",
            "--delete",
        ]);
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
