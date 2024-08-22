use anyhow::{bail, Context};
use roguewave::Session;
use std::env;
use std::sync::Once;

fn setup_logger() {
    static START: Once = Once::new();
    START.call_once(|| {
        if env::var("RUST_LOG").is_err() {
            env::set_var("RUST_LOG", "info")
        }
        env_logger::builder()
            .format_target(false)
            .format_module_path(false)
            .init();
    });
}

#[tokio::test]
async fn integration_test() -> anyhow::Result<()> {
    setup_logger();
    let destination = match env::var("ROGUEWAVE_INTEGRATION_TEST_DESTINATION") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => {
            println!(
                "ROGUEWAVE_INTEGRATION_TEST_DESTINATION env var not specified, skipping integration test"
            );
            return Ok(());
        }
        Err(env::VarError::NotUnicode(value)) => {
            bail!("invalid env var value: {value:?}");
        }
    };

    let mut session = Session::connect(destination).await?;
    test_commands(&mut session).await?;
    test_env(&mut session).await?;
    test_apt(&mut session).await?;
    Ok(())
}

async fn test_commands(session: &mut Session) -> anyhow::Result<()> {
    session
        .command(["bash", "-c", "echo OK > /tmp/1"])
        .run()
        .await?;

    assert_eq!(
        session.command(["cat", "/tmp/1"]).run().await?.stdout,
        "OK\n"
    );

    assert!(!session.path_exists("/tmp/2").await?);
    session
        .command(["bash"])
        .arg("-c")
        .arg("echo OK2 > /tmp/2")
        .run()
        .await?;
    assert!(session.path_exists("/tmp/2").await?);

    session
        .command(["bash"])
        .args(["-c", "echo OK3 > /tmp/3"])
        .run()
        .await?;
    assert_eq!(session.fs().read("/tmp/3").await?, "OK3\n");

    assert_eq!(session.command(["whoami"]).run().await?.stdout, "root\n");
    assert_eq!(
        session.command(["whoami"]).user(None).run().await?.stdout,
        "root\n"
    );

    session.create_user("user1").await?;

    assert_eq!(
        session
            .command(["whoami"])
            .user(Some("user1"))
            .run()
            .await?
            .stdout,
        "user1\n"
    );

    assert_eq!(
        session
            .command(["test2"])
            .prepend_args(["echo", "test1"])
            .run()
            .await?
            .stdout,
        "test1 test2\n"
    );

    assert_eq!(session.command(["cat", "/tmp/1"]).exit_code().await?, 0);
    assert_eq!(session.command(["cat", "/tmp/10"]).exit_code().await?, 1);
    session.command(["cat", "/tmp/10"]).run().await.unwrap_err();
    let failed_output = session
        .command(["cat", "/tmp/10"])
        .allow_failure()
        .run()
        .await?;
    assert_eq!(failed_output.exit_code, 1);
    assert_eq!(failed_output.stdout, "");
    assert_eq!(
        failed_output.stderr,
        "cat: /tmp/10: No such file or directory\n"
    );

    Ok(())
}

async fn test_apt(session: &mut Session) -> anyhow::Result<()> {
    session.apt().update_package_list().await?;
    assert!(!session.apt().is_package_installed("rolldice").await?);
    session.apt().install(&["rolldice"]).await?;
    assert!(session.apt().is_package_installed("rolldice").await?);
    session.command(["rolldice"]).run().await?;

    Ok(())
}

async fn test_env(session: &mut Session) -> anyhow::Result<()> {
    let env = session.env(None).await?;
    assert_eq!(env.get("HOME").unwrap(), "/root");
    assert_eq!(env.get("USER").unwrap(), "root");
    assert_eq!(env.get("PWD").unwrap(), "/root");
    assert_eq!(env.get("SHELL").unwrap(), "/bin/bash");
    assert!(env.contains_key("PATH"));

    assert_eq!(session.home_dir(None).await?, "/root");
    assert_eq!(session.current_user().await?, "root");
    assert_eq!(session.shell(None).await?.as_os_str(), "/bin/bash");
    assert_eq!(get_shell_config(session).await?, "/bin/bash");

    Ok(())
}

async fn get_shell_config(session: &mut Session) -> anyhow::Result<String> {
    session
        .command(["getent", "passwd", "root"])
        .run()
        .await?
        .stdout
        .trim()
        .split(":")
        .nth(6)
        .context("invalid getent output")
        .map(Into::into)
}
