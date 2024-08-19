use std::{
    fmt::Write,
    io::{BufRead, BufReader, Read},
    process::Stdio,
    thread,
};

use anyhow::{anyhow, bail, Context};
use log::log;
use tokio::task::block_in_place;

use crate::CommandOutput;

pub struct LocalCommand {
    command: Vec<String>,
    command_log_level: log::Level,
    stdout_log_level: log::Level,
    stderr_log_level: log::Level,
    allow_failure: bool,
}

impl LocalCommand {
    pub fn new<S: AsRef<str>, I: IntoIterator<Item = S>>(command: I) -> LocalCommand {
        LocalCommand {
            command: command.into_iter().map(|s| s.as_ref().into()).collect(),
            command_log_level: log::Level::Info,
            stdout_log_level: log::Level::Info,
            stderr_log_level: log::Level::Error,
            allow_failure: false,
        }
    }

    pub fn arg(mut self, arg: impl AsRef<str>) -> Self {
        self.command.push(arg.as_ref().into());
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        self.command
            .extend(args.into_iter().map(|arg| arg.as_ref().into()));
        self
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
        log!(
            self.command_log_level,
            "running local command: {:?}",
            self.command
        );
        let mut child = std::process::Command::new(&self.command[0])
            .args(&self.command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stderr_reader = child.stderr.take().context("missing stderr")?;
        let stdout_reader = child.stdout.take().context("missing stdout")?;
        let stderr_task =
            thread::spawn(move || handle_output(stderr_reader, self.stderr_log_level, "stderr: "));
        let stdout_task =
            thread::spawn(move || handle_output(stdout_reader, self.stdout_log_level, "stdout: "));

        let status = block_in_place(|| child.wait())?;
        let exit_code = status.code().context("missing exit code")?;
        if !self.allow_failure && exit_code != 0 {
            bail!("local command failed with exit code {}", exit_code);
        }
        Ok(CommandOutput {
            exit_code,
            stdout: block_in_place(|| stdout_task.join())
                .map_err(|_| anyhow!("local output handler panicked"))??,
            stderr: block_in_place(|| stderr_task.join())
                .map_err(|_| anyhow!("local output handler panicked"))??,
        })
    }
}

fn handle_output(reader: impl Read, log_level: log::Level, prefix: &str) -> anyhow::Result<String> {
    let reader = BufReader::new(reader);
    let mut output = String::new();
    for line in reader.lines() {
        let line = line?;
        writeln!(output, "{}", line)?;
        log!(log_level, "{}{}", prefix, &line);
    }
    Ok(output)
}
