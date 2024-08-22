# roguewave

[![Crates.io][crates-badge]][crates-url]
[![docs.rs][docs-badge]][docs-url]
[![Build Status][actions-badge]][actions-url]
![Crates.io License][license-badge]

[crates-badge]: https://img.shields.io/crates/v/roguewave.svg
[crates-url]: https://crates.io/crates/roguewave
[license-badge]: https://img.shields.io/crates/l/roguewave
[docs-badge]: https://img.shields.io/docsrs/roguewave
[docs-url]: https://docs.rs/roguewave
[actions-badge]: https://img.shields.io/github/actions/workflow/status/Riateche/roguewave/ci.yml?branch=main
[actions-url]: https://github.com/Riateche/roguewave/actions?query=branch%3Amain

`roguewave` is an imperative remote server automation tool.
It allows you to create deployment scripts and automate repetitive
administration tasks.

Unlike many existing tools that achieve similar functionality,
`roguewave` is not based on declarative descriptions and configuration files.
It's a code-first tool, where you use Rust code to describe any process
you implement. This gives you clear control flow, explicit context passing,
simple code deduplication and many more benefits that come with using a modern
high-level language. This also makes deployment and server configuration
more approachable to developers.

`roguewave` doesn't come with many built-in capabilities. Existing built-ins are more like
starting points or examples of what you can achieve, and they make some assumptions
about the remote system that are not universally true (e.g. root access, availability of
`sudo` and `apt`). However, `roguewave` itself can be used with any remote system
that provides SSH and SFTP access.

Instead of relying on built-ins completely, users are encouraged
to create and reuse utility functions that suit their purposes. These utilities can
be shared with others as Rust crates or suggested for merging into `roguewave`.

## Getting started

First, make sure you can connect to your server via SSH without a password. Typically,
you can achieve that by setting up keypair auth and adding your key to ssh-agent.
Next, you can create a `roguewave` session like this:
```rust
use roguewave::Session;
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }
    env_logger::init(); // initialize logger
    let mut session = Session::connect("username@hostname").await?;
    //...
    Ok(())
}
```

The `Session` handle provides access to built-in helpers:
```rust
session.apt().install(&["nginx"]).await?;
session.create_user("alice").await?;
session.fs().write("/home/username/.bashrc", "export PAGER=less\n").await?;
```
You can also run arbitrary commands:
```rust
session.command(["systemctl", "restart", "nginx"]).run().await?;
let uname = session.command(["uname", "-a"]).run().await?.stdout;
```

## Extending `roguewave`

The simplest way to write a custom helper is to create a function:
```rust
use roguewave::Session;

async fn setup_user(session: &mut Session, name: &str) -> anyhow::Result<()> {
    session.create_user(name).await?;
    let home_dir = session.home_dir(Some(name)).await?;
    session.upload(["important_file.txt"], &home_dir, Some(name)).await?;
    Ok(())
}
```
You can create a nicer interface by creating an extension trait:
```rust
use roguewave::Session;

#[async_trait::async_trait]
pub trait SetupUser {
    async fn setup_user(&mut self, name: &str) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
impl SetupUser for Session {
    async fn setup_user(&mut self, name: &str) -> anyhow::Result<()> {
        todo!()
    }
}
```
Alternatively, you can create a struct that provides access to multiple helpers:
```rust
use roguewave::Session;

pub struct Cron<'a>(&'a mut Session);
pub trait GetCron {
    fn cron(&mut self) -> Cron;
}

impl GetCron for Session {
    fn cron(&mut self) -> Cron {
        Cron(self)
    }
}

impl Cron<'_> {
    async fn add_task(&mut self, name: &str) -> anyhow::Result<()> {
        todo!()
    }
    async fn remove_task(&mut self, name: &str) -> anyhow::Result<()> {
        todo!()
    }
}
```

## License
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version 2.0</a>
or <a href="LICENSE-MIT">MIT license</a> at your option. Unless you explicitly state otherwise,
any contribution intentionally submitted for inclusion by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
