//! What every use case does with a session: open a bounded read-only
//! transaction, turn an outcome into rows, read one value.

use anyhow::{Result, anyhow};

use crate::domain::{InputError, ROWS_MAX, RowSet, bounded_rows};
use crate::ports::{Outcome, SqlSession};

pub const TIMEOUT_DEFAULT: i64 = 30;
pub const TIMEOUT_MAX: i64 = 300;
pub const LOCK_TIMEOUT_SECONDS: i64 = 5;

pub fn timeout(value: i64) -> Result<i64, InputError> {
    if (1..=TIMEOUT_MAX).contains(&value) {
        Ok(value)
    } else {
        Err(InputError::OutOfRange {
            what: "timeout_seconds",
            min: 1,
            max: TIMEOUT_MAX,
        })
    }
}

pub fn max_rows(value: i64) -> Result<usize, InputError> {
    if (1..=ROWS_MAX).contains(&value) {
        Ok(usize::try_from(value).unwrap_or(1))
    } else {
        Err(InputError::OutOfRange {
            what: "max_rows",
            min: 1,
            max: ROWS_MAX,
        })
    }
}

/// `BEGIN … READ ONLY` with a statement timeout and a lock timeout, so a
/// read neither writes, nor runs for long, nor waits behind DDL. Both
/// values are validated integers before they are interpolated.
pub async fn begin_read_only(session: &mut dyn SqlSession, timeout_seconds: i64) -> Result<()> {
    let timeout_seconds = timeout(timeout_seconds)?;
    session
        .run("BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY")
        .await?;
    session
        .run(&format!(
            "SET LOCAL statement_timeout = '{timeout_seconds}s'"
        ))
        .await?;
    session
        .run(&format!(
            "SET LOCAL lock_timeout = '{LOCK_TIMEOUT_SECONDS}s'"
        ))
        .await?;
    Ok(())
}

/// Ends a read: nothing to keep.
pub async fn rollback(session: &mut dyn SqlSession) {
    let _ = session.run("ROLLBACK").await;
}

pub fn rows_of(outcome: Outcome, max: usize) -> Result<RowSet> {
    match outcome {
        Outcome::Rows { columns, rows } => Ok(bounded_rows(columns, rows, max)),
        Outcome::Command { tag, .. } => Err(anyhow!("expected rows, the server answered {tag}")),
    }
}

/// One fixed read inside a read-only transaction of its own.
pub async fn fixed_read(
    session: &mut dyn SqlSession,
    sql: &str,
    max: usize,
    timeout_seconds: i64,
) -> Result<RowSet> {
    begin_read_only(session, timeout_seconds).await?;
    let outcome = session.run(sql).await;
    rollback(session).await;
    rows_of(outcome?, max)
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeGateway, rows};
    use super::*;
    use crate::ports::SqlGateway;

    #[tokio::test]
    async fn a_fixed_read_runs_inside_a_bounded_read_only_transaction() {
        let gateway = FakeGateway::answering(vec![("SELECT 1", rows(&["one"], &[&["1"]]))]);
        let mut session = gateway.connect().await.unwrap();
        let set = fixed_read(session.as_mut(), "SELECT 1", 10, 20)
            .await
            .unwrap();
        assert_eq!(set.scalar(), Some("1"));
        assert_eq!(
            gateway.statements(),
            [
                "BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY",
                "SET LOCAL statement_timeout = '20s'",
                "SET LOCAL lock_timeout = '5s'",
                "SELECT 1",
                "ROLLBACK"
            ]
        );
        assert!(begin_read_only(session.as_mut(), 0).await.is_err());
        assert!(begin_read_only(session.as_mut(), 301).await.is_err());
        assert!(max_rows(0).is_err() && max_rows(1001).is_err());
        assert_eq!(max_rows(5).unwrap(), 5);
        assert!(
            rows_of(
                crate::ports::Outcome::Command {
                    tag: "SET".into(),
                    affected: None
                },
                1
            )
            .is_err()
        );
        let failing =
            FakeGateway::answering(vec![("SELECT 1", Err("relation does not exist".into()))]);
        let mut session = failing.connect().await.unwrap();
        let error = fixed_read(session.as_mut(), "SELECT 1", 10, 20)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("does not exist"));
        assert_eq!(
            failing.statements().last().unwrap(),
            "ROLLBACK",
            "a failed read still rolls back"
        );
    }
}
