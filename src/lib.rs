#![warn(missing_docs)]
//! `roguewave` is an imperative remote server automation tool.
//! It allows you to create deployment scripts and automate repetitive
//! administration tasks.
//!
//! Unlike many existing tools that achieve similar functionality,
//! `roguewave` is not based on declarative descriptions and configuration files.
//! It's a code-first tool, where you use Rust code to describe any process
//! you implement. This gives you clear control flow, explicit context passing,
//! simple code deduplication and many more benefits that come with using a modern
//! high-level language. This also makes deployment and server configuration
//! more approachable to developers.
//!
//! `roguewave` doesn't come with many built-in capabilities. Existing built-ins are more like
//! starting points or examples of what you can achieve, and they make some assumptions
//! about the remote system that are not universally true (e.g. root access, availability of
//! `sudo` and `apt`). However, `roguewave` itself can be used with any remote system
//! that provides SSH and SFTP access.
//!
//! Instead of relying on built-ins completely, users are encouraged
//! to create and reuse utility functions that suit their purposes. These utilities can
//! be shared with others as Rust crates or suggested for merging into `roguewave`.
//!
//! # Getting started
//!
//! First, make sure you can connect to your server via SSH without a password. Typically,
//! you can achieve that by setting up keypair auth and adding your key to ssh-agent.
//! Next, you can create a `roguewave` session like this:
//! ```no_run
//! use roguewave::Session;
//! use std::env;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     if env::var("RUST_LOG").is_err() {
//!         env::set_var("RUST_LOG", "info");
//!     }
//!     env_logger::init(); // initialize logger
//!     let mut session = Session::connect("username@hostname").await?;
//!     //...
//!     Ok(())
//! }
//! ```
//!
//! The `Session` handle provides access to built-in helpers:
//! ```no_run
//! # use roguewave::Session;
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! #    let mut session = Session::connect("username@hostname").await?;
//! session.apt().install(&["nginx"]).await?;
//! session.create_user("alice").await?;
//! session.fs().write("/home/username/.bashrc", "export PAGER=less\n").await?;
//! #    Ok(())
//! # }
//! ```
//! You can also run arbitrary commands:
//! ```no_run
//! # use roguewave::Session;
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! #    let mut session = Session::connect("username@hostname").await?;
//! session.command(["systemctl", "restart", "nginx"]).run().await?;
//! let uname = session.command(["uname", "-a"]).run().await?.stdout;
//! #    Ok(())
//! # }
//! ```
//!
//! # Extending `roguewave`
//!
//! The simplest way to write a custom helper is to create a function:
//! ```
//! use roguewave::Session;
//!
//! async fn setup_user(session: &mut Session, name: &str) -> anyhow::Result<()> {
//!     session.create_user(name).await?;
//!     let home_dir = session.home_dir(Some(name)).await?;
//!     session.upload(["important_file.txt"], &home_dir, Some(name)).await?;
//!     Ok(())
//! }
//! ```
//! You can create a nicer interface by creating an extension trait:
//! ```
//! use roguewave::Session;
//!
//! #[async_trait::async_trait]
//! pub trait SetupUser {
//!     async fn setup_user(&mut self, name: &str) -> anyhow::Result<()>;
//! }
//!
//! #[async_trait::async_trait]
//! impl SetupUser for Session {
//!     async fn setup_user(&mut self, name: &str) -> anyhow::Result<()> {
//!         todo!()
//!     }
//! }
//! ```
//! Alternatively, you can create a struct that provides access to multiple helpers:
//! ```
//! use roguewave::Session;
//!
//! pub struct Cron<'a>(&'a mut Session);
//! pub trait GetCron {
//!     fn cron(&mut self) -> Cron;
//! }
//!
//! impl GetCron for Session {
//!     fn cron(&mut self) -> Cron {
//!         Cron(self)
//!     }
//! }
//!
//! impl Cron<'_> {
//!     async fn add_task(&mut self, name: &str) -> anyhow::Result<()> {
//!         todo!()
//!     }
//!     async fn remove_task(&mut self, name: &str) -> anyhow::Result<()> {
//!         todo!()
//!     }
//! }
//! ```

use std::{path::Path, sync::Arc};

use anyhow::Context;
use openssh::{KnownHosts, Stdio};
use openssh_sftp_client::{error::SftpErrorKind, fs::Fs, Error, Sftp};
use type_map::concurrent::TypeMap;

mod command;
mod local;
mod recipes;

pub use command::{Command, CommandOutput};
pub use local::LocalCommand;
pub use recipes::{apt::Apt, postgres::Postgres};

/// A SSH session to a remote host.
pub struct Session {
    user: Option<String>,
    port: Option<u16>,
    destination: String,
    inner: Arc<openssh::Session>,
    #[allow(dead_code)]
    sftp_child: openssh::Child<Arc<openssh::Session>>,
    sftp: Sftp,
    fs: Fs,
    cache: TypeMap,
}

impl Session {
    /// Initialize a SSH session with default configuration.
    ///
    /// The format of `destination` is the same as the `destination` argument to `ssh`. It may be
    /// specified as either `[user@]hostname` or a URI of the form `ssh://[user@]hostname[:port]`.
    ///
    /// If connecting requires interactive authentication based on `STDIN` (such as reading a
    /// password), the connection will fail. Consider setting up keypair-based authentication
    /// instead.
    pub async fn connect(destination: impl AsRef<str>) -> anyhow::Result<Self> {
        let mut builder = openssh::SessionBuilder::default();
        builder.known_hosts_check(KnownHosts::Strict);
        Self::from_openssh_builder(builder, destination).await
    }

    /// Initialize a SSH session from a pre-configured builder.
    /// Allows specifying settings such as port, known hosts policy, etc.
    ///
    /// The format of `destination` is the same as the `destination` argument to `ssh`. It may be
    /// specified as either `[user@]hostname` or a URI of the form `ssh://[user@]hostname[:port]`.
    /// A username or port that is specified in the connection string overrides the one set in the
    /// builder (but does not change the builder).
    ///
    /// If connecting requires interactive authentication based on `STDIN` (such as reading a
    /// password), the connection will fail. Consider setting up keypair-based authentication
    /// instead.
    pub async fn from_openssh_builder(
        builder: openssh::SessionBuilder,
        destination: impl AsRef<str>,
    ) -> anyhow::Result<Self> {
        let (builder, destination) = builder.resolve(destination.as_ref());
        let session = builder.connect_mux(destination).await?;
        let session = Arc::new(session);
        let mut sftp_child = openssh::Session::to_subsystem(session.clone(), "sftp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .await?;

        let sftp = Sftp::new(
            sftp_child.stdin().take().unwrap(),
            sftp_child.stdout().take().unwrap(),
            Default::default(),
        )
        .await?;

        Ok(Session {
            user: builder.get_user().map(Into::into),
            port: builder
                .get_port()
                .map(|s| s.parse())
                .transpose()
                .context("invalid port")?,
            destination: destination.into(),
            inner: session,
            sftp_child,
            fs: sftp.fs(),
            sftp,
            cache: TypeMap::new(),
        })
    }

    /// Access the SFTP subsystem - a file-oriented channel to a remote host.
    ///
    /// See also `fs`.
    pub fn sftp(&mut self) -> &mut Sftp {
        &mut self.sftp
    }

    /// Perform operations on a remote filesystem.
    pub fn fs(&mut self) -> &mut Fs {
        &mut self.fs
    }

    /// Check if a path exists on a remote filesystem.
    pub async fn path_exists(&mut self, path: impl AsRef<Path>) -> anyhow::Result<bool> {
        match self.fs().metadata(path).await {
            Ok(_) => Ok(true),
            Err(Error::SftpError(SftpErrorKind::NoSuchFile, _)) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    /// Access the session cache. The cache may contain values of arbitrary types.
    /// The cache only persists while the `Session` object exists.
    /// This allows to avoid sending repeated commands to the remote host.
    pub fn cache(&mut self) -> &mut TypeMap {
        &mut self.cache
    }
}
