use std::path::Path;

use anyhow::{bail, Context};

use crate::{local, Session};

impl Session {
    /// Upload local files `local_paths` to the remote location `remote_parent_path`.
    ///
    /// Requires `rsync` to be available locally and remotely.
    ///
    /// If `remote_user` is specified, it will be used for the upload
    /// (requires `sudo` available on the remote system).
    ///
    /// Existing remote files will be replaced by new files. When uploading directories,
    /// extraneous files will be deleted from destination directories.
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
        let mut command = local::LocalCommand::new([
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
        if let Some(port) = &self.port {
            command = command.args(["--rsh", &format!("ssh -p {port}")]);
        }
        let destination = if let Some(user) = &self.user {
            format!("{}@{}", user, self.destination)
        } else {
            self.destination.clone()
        };
        command
            .arg(format!(
                "{}:{}",
                destination,
                remote_parent_path
                    .as_ref()
                    .to_str()
                    .context("non-utf8 path")?
            ))
            .run()
            .await?;

        Ok(())
    }
}
