use anyhow::{bail, Context};
use derive_more::From;
use log::log;
use openssh::Stdio;
use std::{
    ffi::{OsStr, OsString},
    fmt,
};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::Session;

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

/// A remote command executor.
///
/// Use `Session::command` or `Session::raw_command` to create a new command.
/// The command, its stdin and stdout will be logged. The logging level can be adjusted.
pub struct Command<'a> {
    session: &'a Session,
    command: Vec<Arg>,
    command_log_level: log::Level,
    stdout_log_level: log::Level,
    stderr_log_level: log::Level,
    allow_failure: bool,
}

impl<'a> Command<'a> {
    /// Append an argument to the command.
    pub fn arg(mut self, arg: impl AsRef<str>) -> Self {
        self.command.push(Arg::escaped(arg));
        self
    }

    /// Append an argument to the command and disable shell escaping for it.
    pub fn raw_arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.command.push(Arg::raw(arg));
        self
    }

    /// Append an argument to the command and prevent logging of it.
    pub fn redacted_arg(mut self, arg: impl AsRef<str>, placeholder: impl AsRef<str>) -> Self {
        self.command.push(Arg {
            kind: ArgKind::escaped(arg),
            display_placeholder: Some(placeholder.as_ref().into()),
        });
        self
    }

    /// Append multiple arguments to the command.
    pub fn args(mut self, args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        self.command
            .extend(args.into_iter().map(|arg| Arg::escaped(arg)));
        self
    }

    /// Append multiple arguments to the command and disable shell escaping for them.
    pub fn raw_args(mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Self {
        self.command
            .extend(args.into_iter().map(|arg| Arg::raw(arg)));
        self
    }

    /// Prepend multiple arguments to the command.
    pub fn prepend_args(mut self, args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut new_args: Vec<_> = args.into_iter().map(|arg| Arg::escaped(arg)).collect();
        new_args.append(&mut self.command);
        self.command = new_args;
        self
    }

    /// Configure the command to be called as another remote user, using `sudo`.
    ///
    /// Equivalent to `prepend_args(["sudo", "--login", "--user", user])`.
    pub fn user(mut self, user: Option<&str>) -> Self {
        if let Some(user) = user {
            self = self.prepend_args(["sudo", "--login", "--user", user]);
        }
        self
    }

    /// Mark the command as possibly expecting a failure.
    /// If `allow_failure` is called before `run`, `run` will no longer return
    /// an error on non-zero exit code.
    pub fn allow_failure(mut self) -> Self {
        self.allow_failure = true;
        self
    }

    /// Execute the command and capture the output.
    ///
    /// By default, non-exit error code will cause `run` to return an error.
    /// If non-exit error code is expected and the output capture is needed,
    /// call `allow_failure` before `run`. If the output capture is not needed,
    /// use `exit_code` instead of `run` for a possibly failing command.
    ///
    /// Non-unicode output in stdout or stderr will result in an error.
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

    /// Execute the command and return the exit code.
    /// Implies `allow_failure`.
    pub async fn exit_code(self) -> anyhow::Result<i32> {
        self.allow_failure()
            .run()
            .await
            .map(|output| output.exit_code)
    }

    /// Lower stdout and stderr logs to `Trace`.
    pub fn hide_all_output(self) -> Self {
        self.hide_stdout().hide_stderr()
    }

    /// Lower stdout logs to `Trace`.
    pub fn hide_stdout(mut self) -> Self {
        self.stdout_log_level = log::Level::Trace;
        self
    }

    /// Set log level for stdout.
    pub fn stdout_log_level(mut self, level: log::Level) -> Self {
        self.stdout_log_level = level;
        self
    }

    /// Lower stderr logs to `Trace`.
    pub fn hide_stderr(mut self) -> Self {
        self.stderr_log_level = log::Level::Trace;
        self
    }

    /// Set log level for stderr.
    pub fn stderr_log_level(mut self, level: log::Level) -> Self {
        self.stderr_log_level = level;
        self
    }

    /// Lower command execution logs to `Trace`.
    pub fn hide_command(mut self) -> Self {
        self.command_log_level = log::Level::Trace;
        self
    }

    /// Set log level for command execution.
    pub fn command_log_level(mut self, level: log::Level) -> Self {
        self.command_log_level = level;
        self
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

/// Information about an output of an executed command.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommandOutput {
    /// Exit code (zero typically means success).
    pub exit_code: i32,
    /// Captured stdout (non-unicode output will result in an error).
    pub stdout: String,
    /// Captured stderr (non-unicode output will result in an error).
    pub stderr: String,
}

impl Session {
    /// Prepare a remote command for execution.
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

    /// Prepare a remote command for execution and disable shell escaping
    /// for the specified portion of the command.
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
}
