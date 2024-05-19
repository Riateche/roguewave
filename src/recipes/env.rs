use std::{collections::BTreeMap, path::Path};

use anyhow::Context;
use async_trait::async_trait;

use crate::Session;

#[async_trait]
pub trait Env {
    async fn env(&mut self) -> anyhow::Result<&BTreeMap<String, String>>;
    async fn home_dir(&mut self) -> anyhow::Result<&Path>;
    async fn current_user(&mut self) -> anyhow::Result<&str>;
    async fn shell(&mut self) -> anyhow::Result<&Path>;
    async fn set_shell(
        &mut self,
        shell: impl AsRef<Path> + Send,
        user: Option<&str>,
    ) -> anyhow::Result<()>;
}

#[async_trait]
impl Env for Session {
    async fn env(&mut self) -> anyhow::Result<&BTreeMap<String, String>> {
        if !self.cache().contains::<EnvCache>() {
            let output = self.command(["env"]).hide_stdout().run().await?;
            let mut env = BTreeMap::new();
            for line in output.stdout.split('\n') {
                if line.is_empty() {
                    continue;
                }
                let line = line.trim_end_matches('\n');
                let mut iter = line.splitn(2, '=');
                let name = iter.next().unwrap();
                let value = iter.next().context("missing '=' in env output")?;
                env.insert(name.to_string(), value.to_string());
            }
            self.cache().insert(EnvCache(env));
        }
        let env = self.cache().get::<EnvCache>().unwrap();
        Ok(&env.0)
    }

    async fn home_dir(&mut self) -> anyhow::Result<&Path> {
        let env = self.env().await?;
        env.get("HOME")
            .context("missing remote env var \"HOME\"")
            .map(Path::new)
    }

    async fn current_user(&mut self) -> anyhow::Result<&str> {
        let env = self.env().await?;
        env.get("USER")
            .context("missing remote env var \"USER\"")
            .map(|s| s.as_str())
    }

    async fn shell(&mut self) -> anyhow::Result<&Path> {
        let env = self.env().await?;
        env.get("SHELL")
            .context("missing remote env var \"SHELL\"")
            .map(Path::new)
    }

    async fn set_shell(
        &mut self,
        shell: impl AsRef<Path> + Send,
        user: Option<&str>,
    ) -> anyhow::Result<()> {
        let shell = shell.as_ref();
        if self.shell().await? != shell {
            self.command(["chsh", "-s", shell.to_str().context("non-utf8 path")?])
                .run()
                .await?;
        }
        Ok(())
    }
}

struct EnvCache(BTreeMap<String, String>);
