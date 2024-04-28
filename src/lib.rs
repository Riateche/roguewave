use std::{path::Path, sync::Arc};

use anyhow::{bail, Context};
use log::info;
use openssh::{KnownHosts::Strict, Stdio};
use openssh_sftp_client::{error::SftpErrorKind, fs::Fs, Sftp};
use tempfile::NamedTempFile;
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
    inner: Arc<openssh::Session>,
    sftp_child: openssh::Child<Arc<openssh::Session>>,
    sftp: Sftp,
    fs: Fs,
}

impl Session {
    pub async fn connect(destination: impl AsRef<str>) -> anyhow::Result<Self> {
        let session = openssh::Session::connect_mux(destination, Strict).await?;
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
            inner: session,
            sftp_child,
            fs: sftp.fs(),
            sftp,
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

    pub fn fs(&mut self) -> &mut Fs {
        &mut self.fs
    }

    pub async fn upload(
        &mut self,
        local_path: impl AsRef<Path>,
        remote_path: impl AsRef<Path>,
    ) -> anyhow::Result<()> {
        let local_path = local_path.as_ref();
        let remote_path = remote_path.as_ref();

        let local_parent = local_path.parent().context("failed to get parent dir")?;
        let local_last_name = local_path.file_name().context("failed to get file name")?;
        let remote_parent = remote_path.parent().context("failed to get parent dir")?;
        let remote_last_name = remote_path.file_name().context("failed to get file name")?;
        if local_last_name != remote_last_name {
            bail!("changing last name on upload is unsupported"); // TODO
        }

        let local_archive_path = NamedTempFile::new()?.into_temp_path();
        let status = std::process::Command::new("tar")
            .args(["--create", "--gzip", "--file"])
            .arg(&local_archive_path)
            .arg("--directory")
            .arg(&local_parent)
            .arg(&local_last_name)
            .status()?;
        if !status.success() {
            bail!("local tar command failed");
        }
        let content = fs_err::read(&local_archive_path)?;
        // TODO: proper temp file generation
        let remote_archive_path = "/tmp/1.tar.gz";
        self.fs.write(&remote_archive_path, content).await?;

        self.command([
            "tar",
            "--extract",
            "--file",
            remote_archive_path,
            "--directory",
            remote_parent.to_str().context("non-utf8 path")?,
        ])
        .run()
        .await?;

        Ok(())
    }
}
