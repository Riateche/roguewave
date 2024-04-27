use anyhow::bail;

use crate::Session;

pub async fn is_package_installed(session: &Session, package: &str) -> anyhow::Result<bool> {
    let code = session
        .command(["dpkg", "--status", package])
        .exit_code()
        .await?;
    match code {
        0 => Ok(true),
        1 => Ok(false),
        _ => bail!("command failed"),
    }
}
