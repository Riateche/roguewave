use anyhow::bail;

use crate::Session;

pub async fn is_package_installed(session: &Session, package: &str) -> anyhow::Result<bool> {
    let output = session
        .command([
            "dpkg-query",
            "--show",
            "--showformat='${db:Status-Status}'",
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

pub async fn update_package_list(session: &Session) -> anyhow::Result<()> {
    session.command(["apt-get", "update"]).run().await?;
    Ok(())
}

async fn update_package_list_unless_cached(session: &mut Session) -> anyhow::Result<()> {
    if !session.cache().contains::<PackageListUpdated>() {
        update_package_list(session).await?;
        session.cache().insert(PackageListUpdated);
    }
    Ok(())
}

struct PackageListUpdated;
