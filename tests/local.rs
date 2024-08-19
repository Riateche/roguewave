use std::{fs, path::Path};

use roguewave::LocalCommand;

#[tokio::test(flavor = "multi_thread")]
async fn test_local_command() -> anyhow::Result<()> {
    if Path::new("/tmp/21").exists() {
        println!("OK1");
        fs::remove_file("/tmp/21")?;
    }
    println!("OK2");
    LocalCommand::new(["bash", "-c", "echo OK > /tmp/21"])
        .run()
        .await?;
    println!("OK3");
    assert!(Path::new("/tmp/21").exists());
    LocalCommand::new(["rm", "/tmp/21"]).run().await?;
    println!("OK4");
    assert!(!Path::new("/tmp/21").exists());

    let output = LocalCommand::new(["echo"])
        .arg("arg1")
        .args(["arg2", "arg3"])
        .run()
        .await?;
    println!("OK5");
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, "arg1 arg2 arg3\n");
    assert_eq!(output.stderr, "");

    LocalCommand::new(["cat", "/tmp/21"])
        .run()
        .await
        .unwrap_err();
    println!("OK6");
    let failed_output = LocalCommand::new(["cat", "/tmp/21"])
        .allow_failure()
        .run()
        .await?;
    println!("OK7");
    assert_eq!(failed_output.exit_code, 1);
    assert_eq!(failed_output.stdout, "");
    assert_eq!(
        failed_output.stderr,
        "cat: /tmp/21: No such file or directory\n"
    );

    Ok(())
}
