use std::time::{Duration, SystemTime};

use anyhow::bail;
use async_trait::async_trait;
use log::info;

use crate::Session;

const AUTO_UPDATE_PERIOD: Duration = Duration::from_secs(3600);

#[async_trait]
pub trait Apt {
    async fn is_package_installed(&self, package: &str) -> anyhow::Result<bool>;
    async fn install_package(&mut self, package: &str) -> anyhow::Result<()>;
    async fn upgrade_system(&mut self) -> anyhow::Result<()>;
    async fn update_package_list(&mut self) -> anyhow::Result<()>;
}

#[async_trait]
impl Apt for Session {
    async fn is_package_installed(&self, package: &str) -> anyhow::Result<bool> {
        let output = self
            .command([
                "dpkg-query",
                "--show",
                "--showformat=${db:Status-Status}",
                package,
            ])
            .hide_all_output()
            .allow_failure()
            .run()
            .await?;
        match output.exit_code {
            0 => Ok(output.stdout == "installed"),
            1 => Ok(false),
            _ => bail!("command failed"),
        }
    }

    async fn install_package(&mut self, package: &str) -> anyhow::Result<()> {
        if !self.is_package_installed(package).await? {
            update_package_list_unless_cached(self).await?;
            self.command(["apt-get", "install", "--yes", package])
                .run()
                .await?;
        }
        Ok(())
    }

    async fn upgrade_system(&mut self) -> anyhow::Result<()> {
        update_package_list_unless_cached(self).await?;
        self.command(["apt-get", "dist-upgrade", "--yes"])
            .run()
            .await?;
        Ok(())
    }

    async fn update_package_list(&mut self) -> anyhow::Result<()> {
        self.command(["apt-get", "update"]).run().await?;
        self.cache().insert(PackageListUpdated);
        Ok(())
    }
}

async fn update_package_list_unless_cached(session: &mut Session) -> anyhow::Result<()> {
    if !session.cache().contains::<PackageListUpdated>() {
        if let Some(last_updated) = last_update_time(session).await {
            let elapsed = last_updated.elapsed()?;
            if elapsed < AUTO_UPDATE_PERIOD {
                info!(
                    "apt-get update was executed {} s ago, skipping",
                    elapsed.as_secs()
                );
                session.cache().insert(PackageListUpdated);
                return Ok(());
            }
        }
        session.update_package_list().await?;
    }
    Ok(())
}

async fn last_update_time(session: &mut Session) -> Option<SystemTime> {
    let metadata = session
        .fs()
        .metadata("/var/lib/apt/periodic/update-success-stamp")
        .await
        .ok()?;
    Some(metadata.modified()?.as_system_time())
}

struct PackageListUpdated;
