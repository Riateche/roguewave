use std::{collections::BTreeMap, path::Path};

use anyhow::Context;
use async_trait::async_trait;

use crate::Session;

#[async_trait]
pub trait Env {
    async fn env(&mut self, user: Option<&str>) -> anyhow::Result<&BTreeMap<String, String>>;
    async fn home_dir(&mut self, user: Option<&str>) -> anyhow::Result<&str>;
    async fn current_user(&mut self) -> anyhow::Result<&str>;
    async fn shell(&mut self, user: Option<&str>) -> anyhow::Result<&Path>;
    async fn set_shell(
        &mut self,
        shell: impl AsRef<Path> + Send,
        user: Option<&str>,
    ) -> anyhow::Result<()>;
}

#[async_trait]
impl Env for Session {
    async fn env(&mut self, user: Option<&str>) -> anyhow::Result<&BTreeMap<String, String>> {
        let cache_has_user = self
            .cache()
            .get::<EnvCache>()
            .map_or(false, |c| c.has_user(user));

        if !cache_has_user {
            let output = self.command(["env"]).user(user).hide_stdout().run().await?;
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
            let cache = self
                .cache()
                .entry::<EnvCache>()
                .or_insert_with(EnvCache::default);
            if let Some(user) = user {
                cache.other_users.insert(user.into(), env);
            } else {
                cache.current_user = Some(env);
            }
        }

        let cache = self.cache().get::<EnvCache>().unwrap();
        if let Some(user) = user {
            Ok(cache.other_users.get(user).unwrap())
        } else {
            Ok(cache.current_user.as_ref().unwrap())
        }
    }

    async fn home_dir(&mut self, user: Option<&str>) -> anyhow::Result<&str> {
        let env = self.env(user).await?;
        env.get("HOME")
            .context("missing remote env var \"HOME\"")
            .map(|s| s.as_str())
    }

    async fn current_user(&mut self) -> anyhow::Result<&str> {
        let env = self.env(None).await?;
        env.get("USER")
            .context("missing remote env var \"USER\"")
            .map(|s| s.as_str())
    }

    async fn shell(&mut self, user: Option<&str>) -> anyhow::Result<&Path> {
        let env = self.env(user).await?;
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
        if self.shell(user).await? != shell {
            let mut command =
                self.command(["chsh", "-s", shell.to_str().context("non-utf8 path")?]);
            if let Some(user) = user {
                command = command.arg(user);
            }
            command.run().await?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct EnvCache {
    current_user: Option<BTreeMap<String, String>>,
    other_users: BTreeMap<String, BTreeMap<String, String>>,
}

impl EnvCache {
    fn has_user(&self, user: Option<&str>) -> bool {
        if let Some(user) = user {
            self.other_users.contains_key(user)
        } else {
            self.current_user.is_some()
        }
    }
}
