use anyhow::{bail, Result};
use format_sql_query::QuotedData;

use crate::Session;

impl Session {
    /// Execute PostgreSQL commands.
    pub fn postgres(&mut self) -> Postgres {
        Postgres(self)
    }
}

/// Provides access to PostgreSQL commands.
pub struct Postgres<'a>(&'a mut Session);

impl<'a> Postgres<'a> {
    /// Create a PostgreSQL user with the specified password.
    ///
    /// Note: if the user with the specified name already exists, its password will not be changed.
    pub async fn create_user_with_password(&mut self, user: &str, password: &str) -> Result<()> {
        if !user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            bail!("invalid postgres user name");
        }

        let user_exists = self
            .0
            .command([
                "psql",
                "--tuples-only",
                "--command",
                &format!(
                    "SELECT 1 FROM pg_roles WHERE rolname = {}",
                    QuotedData(user)
                ),
            ])
            .prepend_args(["sudo", "--user", "postgres", "--login"])
            .hide_command()
            .hide_stdout()
            .run()
            .await?
            .stdout
            .contains('1');

        if !user_exists {
            self.0
                .command(["psql", "--command"])
                .redacted_arg(
                    format!(
                        "CREATE USER {} WITH PASSWORD {}",
                        user,
                        QuotedData(password)
                    ),
                    format!(
                        "CREATE USER {} WITH PASSWORD {}",
                        user,
                        QuotedData("<redacted>")
                    ),
                )
                .prepend_args(["sudo", "--user", "postgres", "--login"])
                .run()
                .await?;
        }
        Ok(())
    }

    /// Create a PostgreSQL database.
    pub async fn create_database(&mut self, name: &str) -> Result<()> {
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        {
            bail!("invalid postgres database name");
        }

        let db_exists = self
            .0
            .command([
                "psql",
                "--tuples-only",
                "--command",
                &format!(
                    "SELECT 1 FROM pg_database WHERE datname = {}",
                    QuotedData(name)
                ),
            ])
            .prepend_args(["sudo", "--user", "postgres", "--login"])
            .hide_command()
            .hide_stdout()
            .run()
            .await?
            .stdout
            .contains('1');

        if !db_exists {
            self.0
                .command(["psql", "--command", &format!("CREATE DATABASE {}", name)])
                .prepend_args(["sudo", "--user", "postgres", "--login"])
                .run()
                .await?;
        }
        Ok(())
    }

    /// Grant all privileges on `database` to `user`.
    pub async fn grant_all_privileges(&mut self, database: &str, user: &str) -> Result<()> {
        if !user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            bail!("invalid postgres user name");
        }
        if !database
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        {
            bail!("invalid postgres database name");
        }

        self.0
            .command([
                "psql",
                "--command",
                &format!("GRANT ALL PRIVILEGES ON DATABASE {} TO {}", database, user),
            ])
            .prepend_args(["sudo", "--user", "postgres", "--login"])
            .run()
            .await?;
        Ok(())
    }
}
