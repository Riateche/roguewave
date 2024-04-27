use anyhow::{bail, Context};
use log::info;
use openssh::{KnownHosts::Strict, Stdio};
use tokio::io::{AsyncRead, AsyncReadExt};

pub mod recipes;

pub struct Command<'a> {
    session: &'a Session,
    command: Vec<String>,
    show_stdout: bool,
    show_stderr: bool,
    raw: bool,
}

impl<'a> Command<'a> {
    pub async fn run(self) -> anyhow::Result<CommandOutput> {
        let output = self.run_internal().await?;
        if output.exit_code != 0 {
            bail!("failed with exit code {}", output.exit_code);
        }
        Ok(output)
    }

    pub async fn exit_code(self) -> anyhow::Result<i32> {
        self.run_internal().await.map(|output| output.exit_code)
    }

    pub fn hide_output(mut self) -> Self {
        self.show_stdout = false;
        self.show_stderr = false;
        self
    }

    async fn run_internal(self) -> anyhow::Result<CommandOutput> {
        if self.command.is_empty() {
            bail!("cannot run empty command");
        }
        info!("running {:?}", self.command);
        let mut cmd = if self.raw {
            self.session.inner.raw_command(&self.command[0])
        } else {
            self.session.inner.command(&self.command[0])
        };
        if self.raw {
            cmd.raw_args(&self.command[1..]);
        } else {
            cmd.args(&self.command[1..]);
        }
        cmd.stderr(Stdio::piped());
        cmd.stdout(Stdio::piped());
        let mut child = cmd.spawn().await?;
        let stderr_reader = child.stderr().take().context("missing stderr")?;
        let stdout_reader = child.stdout().take().context("missing stdout")?;
        let stderr_task = tokio::spawn(handle_output(stderr_reader, self.show_stderr, "stderr: "));
        let stdout_task = tokio::spawn(handle_output(stdout_reader, self.show_stdout, "stdout: "));
        let status = child.wait().await?;
        let exit_code = status.code().context("missing exit code")?;
        Ok(CommandOutput {
            exit_code,
            stdout: stdout_task.await??,
            stderr: stderr_task.await??,
        })
    }
}

async fn handle_output(
    reader: impl AsyncRead,
    print: bool,
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
            if print {
                info!("{}{}", prefix, &line[..line.len() - 1]);
            }
            output.push_str(line);
            vec.drain(..=index);
        }
    }
    if !vec.is_empty() {
        let line = std::str::from_utf8(&vec)?;
        if print {
            info!("{}{}[eof]", prefix, line);
        }
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
    inner: openssh::Session,
}

impl Session {
    pub async fn connect(destination: impl AsRef<str>) -> anyhow::Result<Self> {
        Ok(Session {
            inner: openssh::Session::connect(destination, Strict).await?,
        })
    }

    pub fn command<S: AsRef<str>, I: IntoIterator<Item = S>>(&self, command: I) -> Command<'_> {
        Command {
            session: self,
            command: command.into_iter().map(|s| s.as_ref().into()).collect(),
            show_stdout: true,
            show_stderr: true,
            raw: false,
        }
    }
}
