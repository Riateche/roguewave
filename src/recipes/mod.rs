use std::path::Path;

use anyhow::Context;

use crate::Session;

pub mod apt;
pub mod env;

pub async fn set_shell(session: &mut Session, shell: impl AsRef<Path>) -> anyhow::Result<()> {
    let shell = shell.as_ref();
    if env::shell(session).await? != shell {
        session
            .command(["chsh", "-s", shell.to_str().context("non-utf8 path")?])
            .run()
            .await?;
    }
    Ok(())
}
