use std::time::{Duration, SystemTime};

use anyhow::bail;
use log::info;

use crate::Session;

const AUTO_UPDATE_PERIOD: Duration = Duration::from_secs(3600);

pub async fn is_package_installed(session: &Session, package: &str) -> anyhow::Result<bool> {
    let output = session
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

pub async fn install_package(session: &mut Session, package: &str) -> anyhow::Result<()> {
    if !is_package_installed(session, package).await? {
        update_package_list_unless_cached(session).await?;
        session
            .command(["apt-get", "install", "--yes", package])
            .run()
            .await?;
    }
    Ok(())
}

pub async fn upgrade_system(session: &mut Session) -> anyhow::Result<()> {
    update_package_list_unless_cached(session).await?;
    session
        .command(["apt-get", "dist-upgrade", "--yes"])
        .run()
        .await?;
    Ok(())
}

pub async fn update_package_list(session: &mut Session) -> anyhow::Result<()> {
    session.command(["apt-get", "update"]).run().await?;
    session.cache().insert(PackageListUpdated);
    Ok(())
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
        update_package_list(session).await?;
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
