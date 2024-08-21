use anyhow::{bail, Context, Result};
use log::{debug, info};

use crate::Session;

impl Session {
    /// Check if the user `name` exists on the remote system.
    pub async fn user_exists(&self, name: &str) -> Result<bool> {
        let code = self
            .command(["id", "--user", name])
            .hide_command()
            .hide_all_output()
            .exit_code()
            .await?;
        match code {
            0 => Ok(true),
            1 => Ok(false),
            _ => bail!("unexpected exit code"),
        }
    }

    /// Create a user and its home directory on the remote system.
    pub async fn create_user(&self, name: &str) -> Result<()> {
        if self.user_exists(name).await? {
            debug!("user {name:?} already exists");
            return Ok(());
        }
        self.command(["useradd", "--create-home", name])
            .run()
            .await?;
        info!("created user {name:?}");
        Ok(())
    }

    /// Fetch remote user ID by name.
    pub async fn user_id(&self, name: &str) -> Result<u32> {
        self.command(["id", "--user", name])
            .hide_command()
            .hide_stdout()
            .run()
            .await?
            .stdout
            .trim()
            .parse()
            .context("failed to parse user id")
    }
}
