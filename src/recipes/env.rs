use std::{collections::BTreeMap, path::Path};

use anyhow::Context;

use crate::Session;

pub async fn env(session: &mut Session) -> anyhow::Result<&BTreeMap<String, String>> {
    if !session.cache().contains::<Env>() {
        let output = session.command(["env"]).hide_stdout().run().await?;
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
        session.cache().insert(Env(env));
    }
    let env = session.cache().get::<Env>().unwrap();
    Ok(&env.0)
}

struct Env(BTreeMap<String, String>);

pub async fn home_dir(session: &mut Session) -> anyhow::Result<&Path> {
    let env = env(session).await?;
    env.get("HOME")
        .context("missing remote env var \"HOME\"")
        .map(Path::new)
}
