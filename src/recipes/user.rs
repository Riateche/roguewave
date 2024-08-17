use anyhow::{bail, Result};
use async_trait::async_trait;
use log::{debug, info};

use crate::Session;

#[async_trait]
pub trait User {
    async fn user_exists(&self, name: &str) -> Result<bool>;
    async fn create_user(&self, name: &str) -> Result<()>;
}

#[async_trait]
impl User for Session {
    async fn user_exists(&self, name: &str) -> Result<bool> {
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

    async fn create_user(&self, name: &str) -> Result<()> {
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
}
