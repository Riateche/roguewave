use std::time::{Duration, SystemTime};

use anyhow::bail;
use log::info;

use crate::Session;

const AUTO_UPDATE_PERIOD: Duration = Duration::from_secs(3600);

impl Session {
    /// Execute apt package management commands.
    pub fn apt(&mut self) -> Apt {
        Apt(self)
    }
}

pub struct Apt<'a>(&'a mut Session);

impl<'a> Apt<'a> {
    /// Update package list.
    pub async fn update_package_list(&mut self) -> anyhow::Result<()> {
        self.0.command(["apt-get", "update"]).run().await?;
        self.0.cache().insert(PackageListUpdated);
        Ok(())
    }

    /// Check if a package is installed.
    pub async fn is_package_installed(&self, package: &str) -> anyhow::Result<bool> {
        let output = self
            .0
            .command([
                "dpkg-query",
                "--show",
                "--showformat=${db:Status-Status}",
                package,
            ])
            .hide_command()
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

    /// Install specified packages.
    pub async fn install(&mut self, packages: &[&str]) -> anyhow::Result<()> {
        let mut new_packages = Vec::new();
        for package in packages {
            if !self.is_package_installed(package).await? {
                new_packages.push(package);
            }
        }
        if !new_packages.is_empty() {
            self.0
                .command(["apt-get", "install", "--yes"])
                .args(new_packages)
                .run()
                .await?;
        }
        Ok(())
    }

    /// Upgrade the system. Update package list before the upgrade if necessary.
    pub async fn upgrade_system(&mut self) -> anyhow::Result<()> {
        update_package_list_unless_cached(self.0).await?;
        self.0
            .command([
                "DEBIAN_FRONTEND=noninteractive",
                "apt-get",
                "dist-upgrade",
                "--yes",
            ])
            .run()
            .await?;
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
        session.apt().update_package_list().await?;
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
