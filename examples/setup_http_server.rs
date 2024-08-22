//! This example sets up a simple web server in a docker container.
//!
//! Basic usage:
//! ```
//! cargo run --example setup_http_server -- ssh://root@127.0.0.1:2222 setup
//! ```
//!
//! You can run this example against a local docker container by
//! running `./run_integration_tests.sh` from the project root.

use std::{env, path::Path};

use clap::Parser;
use roguewave::Session;

#[derive(Debug, Parser)]
struct Command {
    /// Remote server to use.
    /// The format is the same as the `destination` argument to `ssh`. It may be
    /// specified as either `[user@]hostname` or a URI of the form `ssh://[user@]hostname[:port]`.
    destination: String,
    #[command(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, Parser)]
enum Subcommand {
    /// Configure web server.
    Setup,
    /// Stop web server.
    Stop,
    /// Start web server.
    Start,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Enable logging by default.
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info")
    }
    // Set up logger.
    env_logger::builder()
        .format_target(false)
        .format_module_path(false)
        .init();

    // Change directory so that we can specify local paths relative to the examples dir.
    env::set_current_dir(Path::new(&env::var("CARGO_MANIFEST_DIR")?).join("examples"))?;

    // Parse command line arguments.
    let command: Command = Command::parse();
    // Connect to the specified server.
    let mut session = Session::connect(&command.destination).await?;
    match command.subcommand {
        Subcommand::Setup => {
            setup(&mut session).await?;
        }
        Subcommand::Stop => {
            // This example works with a docker container so
            // these commands are somewhat unusual. On a normal system,
            // it would be better to use `systemctl` instead.
            session.command(["pkill", "nginx"]).run().await?;
        }
        Subcommand::Start => {
            session.command(["/usr/sbin/nginx"]).run().await?;
        }
    }
    Ok(())
}

async fn setup(session: &mut Session) -> anyhow::Result<()> {
    session.apt().update_package_list().await?;
    session.apt().install(&["nginx", "rsync"]).await?;
    if session
        .path_exists("/etc/nginx/sites-enabled/default")
        .await?
    {
        session
            .fs()
            .remove_file("/etc/nginx/sites-enabled/default")
            .await?;
    }
    // Upload a virtual host config file.
    session
        .upload(["http_server.conf"], "/etc/nginx/sites-enabled", None)
        .await?;
    // Upload files for the web server.
    session.upload(["files"], "/var/www", None).await?;
    // That would normally be `systemctl reload nginx`.
    session.command(["/usr/sbin/nginx"]).run().await?;
    Ok(())
}
