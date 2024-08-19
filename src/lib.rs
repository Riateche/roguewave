use std::{path::Path, sync::Arc};

use openssh::{KnownHosts, Stdio};
use openssh_sftp_client::{error::SftpErrorKind, fs::Fs, Error, Sftp};
use type_map::concurrent::TypeMap;

mod command;
mod local;
mod recipes;

pub use command::{Command, CommandOutput};
pub use local::LocalCommand;

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
    pub async fn from_openssh_builder(
        builder: openssh::SessionBuilder,
        destination: impl AsRef<str>,
    ) -> anyhow::Result<Self> {
        let session = builder.connect_mux(destination.as_ref()).await?;
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

    pub async fn connect(destination: impl AsRef<str>) -> anyhow::Result<Self> {
        let mut builder = openssh::SessionBuilder::default();
        builder.known_hosts_check(KnownHosts::Strict);
        Self::from_openssh_builder(builder, destination).await
    }

    pub fn sftp(&mut self) -> &mut Sftp {
        &mut self.sftp
    }

    pub fn fs(&mut self) -> &mut Fs {
        &mut self.fs
    }

    pub async fn path_exists(&mut self, path: impl AsRef<Path>) -> anyhow::Result<bool> {
        match self.fs().metadata(path).await {
            Ok(_) => Ok(true),
            Err(Error::SftpError(SftpErrorKind::NoSuchFile, _)) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    pub fn cache(&mut self) -> &mut TypeMap {
        &mut self.cache
    }
}
