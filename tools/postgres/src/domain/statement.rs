use super::{InputError, WriteLevel};

/// What a statement is, from its first words once strings and comments
/// are out of the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// SELECT, WITH, TABLE, VALUES.
    Select,
    Explain,
    Show,
    Vacuum,
    Analyze,
    Reindex,
    CreateIndex {
        concurrently: bool,
    },
    Alter {
        system: bool,
    },
    RefreshView,
    Cluster,
    /// INSERT, UPDATE, DELETE, MERGE.
    Dml(String),
    Drop,
    Truncate,
    /// Other CREATE, GRANT, REVOKE, DO, CALL, COMMENT, and the rest.
    Other(String),
    /// BEGIN, COMMIT, ROLLBACK, SET, RESET, COPY, LISTEN: never run here.
    Refused(String),
}

impl Kind {
    #[must_use]
    pub fn is_read(&self) -> bool {
        matches!(self, Self::Select | Self::Explain | Self::Show)
    }

    /// The write level a statement needs; `None` for reads and refused ones.
    #[must_use]
    pub fn level(&self) -> Option<WriteLevel> {
        match self {
            Self::Select | Self::Explain | Self::Show | Self::Refused(_) => None,
            Self::Vacuum
            | Self::Analyze
            | Self::Reindex
            | Self::CreateIndex { concurrently: true }
            | Self::Alter { system: false }
            | Self::RefreshView
            | Self::Cluster => Some(WriteLevel::Maintenance),
            _ => Some(WriteLevel::All),
        }
    }

    /// Statements that refuse a transaction block: run alone, autocommit.
    #[must_use]
    pub fn outside_transaction(&self) -> bool {
        matches!(
            self,
            Self::Vacuum
                | Self::CreateIndex { concurrently: true }
                | Self::Reindex
                | Self::Alter { system: true }
                | Self::Cluster
                | Self::RefreshView
        )
    }

    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Select => "SELECT".into(),
            Self::Explain => "EXPLAIN".into(),
            Self::Show => "SHOW".into(),
            Self::Vacuum => "VACUUM".into(),
            Self::Analyze => "ANALYZE".into(),
            Self::Reindex => "REINDEX".into(),
            Self::CreateIndex { concurrently: true } => "CREATE INDEX CONCURRENTLY".into(),
            Self::CreateIndex {
                concurrently: false,
            } => "CREATE INDEX".into(),
            Self::Alter { system: true } => "ALTER SYSTEM".into(),
            Self::Alter { system: false } => "ALTER".into(),
            Self::RefreshView => "REFRESH MATERIALIZED VIEW".into(),
            Self::Cluster => "CLUSTER".into(),
            Self::Dml(verb) | Self::Other(verb) | Self::Refused(verb) => verb.clone(),
            Self::Drop => "DROP".into(),
            Self::Truncate => "TRUNCATE".into(),
        }
    }
}

/// Functions a read must not call: they act on the server, not on data.
const DENIED_FUNCTIONS: [&str; 20] = [
    "pg_terminate_backend",
    "pg_cancel_backend",
    "pg_reload_conf",
    "pg_switch_wal",
    "pg_promote",
    "pg_rotate_logfile",
    "pg_create_physical_replication_slot",
    "pg_create_logical_replication_slot",
    "pg_drop_replication_slot",
    "pg_logical_emit_message",
    "lo_import",
    "lo_export",
    "lo_unlink",
    "pg_read_file",
    "pg_read_binary_file",
    "pg_ls_dir",
    "pg_stat_file",
    "pg_file_write",
    "dblink",
    "dblink_exec",
];

/// One SQL statement, classified. Strings, quoted identifiers, dollar
/// quotes and comments are walked so a `;` or a function name inside them
/// does not count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    text: String,
    code: String,
    kind: Kind,
}

impl Statement {
    pub fn parse(raw: &str) -> Result<Self, InputError> {
        let text = raw.trim().trim_end_matches(';').trim().to_string();
        if text.is_empty() {
            return Err(InputError::Empty("sql"));
        }
        let (code, statements) = scrub(&text)?;
        if statements > 1 {
            return Err(InputError::MultipleStatements(statements));
        }
        let kind = classify(&code);
        Ok(Self { text, code, kind })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    /// For `query`: a read that calls none of the denied functions.
    pub fn require_read(&self) -> Result<(), InputError> {
        if !self.kind.is_read() {
            return Err(InputError::NotARead(self.kind.label()));
        }
        self.require_no_denied_function()
    }

    pub fn require_no_denied_function(&self) -> Result<(), InputError> {
        let lower = self.code.to_ascii_lowercase();
        for function in DENIED_FUNCTIONS {
            if let Some(at) = lower.find(function) {
                let before = lower[..at].chars().next_back();
                let after = lower[at + function.len()..].trim_start().chars().next();
                let word_start = before.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
                if word_start && after == Some('(') {
                    return Err(InputError::DeniedFunction(function.into()));
                }
            }
        }
        Ok(())
    }

    /// For `execute`: the level the statement needs against the one set.
    pub fn require_write(&self, level: WriteLevel) -> Result<(), InputError> {
        if level == WriteLevel::Off {
            return Err(InputError::WritesDisabled);
        }
        if self.kind.is_read() {
            return Err(InputError::IsARead(self.kind.label()));
        }
        if let Kind::Refused(verb) = &self.kind {
            return Err(InputError::Invalid {
                what: "sql",
                value: verb.clone(),
                rule: "transaction control, SET, COPY and LISTEN are not run here",
            });
        }
        if self.kind
            == (Kind::CreateIndex {
                concurrently: false,
            })
            && level == WriteLevel::Maintenance
        {
            return Err(InputError::IndexNotConcurrent);
        }
        if self.kind == (Kind::Alter { system: false }) && level == WriteLevel::Maintenance {
            // The catalog bot's maintenance never drops and only touches
            // relations: ALTER … DROP, ALTER ROLE, ALTER DATABASE and the
            // like belong to the all level.
            let words: Vec<String> = self
                .code
                .split(|c: char| c.is_whitespace() || c == '(' || c == ',')
                .filter(|w| !w.is_empty())
                .map(|w| w.to_ascii_uppercase())
                .collect();
            let object = words.get(1).map(String::as_str).unwrap_or("");
            let relation = matches!(
                object,
                "TABLE" | "INDEX" | "SEQUENCE" | "VIEW" | "MATERIALIZED"
            );
            if !relation {
                return Err(InputError::NeedsAll {
                    kind: format!("ALTER {object}"),
                });
            }
            if words.iter().any(|w| w == "DROP") {
                return Err(InputError::NeedsAll {
                    kind: "ALTER … DROP".into(),
                });
            }
        }
        match self.kind.level() {
            Some(needed) if needed > level => Err(InputError::NeedsAll {
                kind: self.kind.label(),
            }),
            _ => Ok(()),
        }
    }

    /// The first table-like name after the verb, for a preview: the word
    /// after INDEX … ON, VACUUM, ANALYZE, REINDEX TABLE, ALTER TABLE,
    /// INSERT INTO, UPDATE, DELETE FROM, TRUNCATE, DROP TABLE.
    #[must_use]
    pub fn target(&self) -> Option<String> {
        let words: Vec<String> = self
            .code
            .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
            .filter(|w| !w.is_empty())
            .map(|w| w.to_string())
            .collect();
        let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
        let after = |keys: &[&str]| -> Option<String> {
            let mut i = 0;
            for key in keys {
                i = lower[i..].iter().position(|w| w == key)? + i + 1;
            }
            words.get(i).cloned()
        };
        let skip_options = |mut i: usize| -> Option<String> {
            while let Some(w) = lower.get(i) {
                if matches!(
                    w.as_str(),
                    "verbose"
                        | "full"
                        | "freeze"
                        | "analyze"
                        | "concurrently"
                        | "only"
                        | "if"
                        | "exists"
                        | "not"
                        | "table"
                        | "index"
                        | "materialized"
                        | "view"
                ) {
                    i += 1;
                } else {
                    return words.get(i).cloned();
                }
            }
            None
        };
        match &self.kind {
            Kind::CreateIndex { .. } => {
                after(&["on"]).map(|t| t.trim_end_matches("using").to_string())
            }
            Kind::Vacuum
            | Kind::Analyze
            | Kind::Cluster
            | Kind::RefreshView
            | Kind::Reindex
            | Kind::Truncate => skip_options(1),
            Kind::Alter { system: false } | Kind::Drop => skip_options(1),
            Kind::Dml(verb) => match verb.as_str() {
                "INSERT" => after(&["into"]),
                "UPDATE" => skip_options(1),
                "DELETE" => after(&["from"]),
                _ => after(&["into"]),
            },
            _ => None,
        }
        .map(|t| t.replace('"', ""))
        .filter(|t| !t.is_empty() && !t.starts_with('$'))
    }
}

/// The text with every string literal and quoted identifier replaced by
/// `?` and every comment removed, and how many statements it holds.
fn scrub(text: &str) -> Result<(String, usize), InputError> {
    let chars: Vec<char> = text.chars().collect();
    let mut code = String::with_capacity(text.len());
    let mut statements = 1usize;
    let mut saw_code = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '-' if next == Some('-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                let mut depth = 1;
                i += 2;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if depth > 0 {
                    return Err(InputError::Unterminated("comment"));
                }
                code.push(' ');
            }
            '\'' => {
                let trimmed = code.trim_end();
                let escaped = trimmed.ends_with(['E', 'e'])
                    && !trimmed[..trimmed.len() - 1]
                        .ends_with(|p: char| p.is_alphanumeric() || p == '_');
                i += 1;
                loop {
                    let Some(&c) = chars.get(i) else {
                        return Err(InputError::Unterminated("string"));
                    };
                    if escaped && c == '\\' {
                        i += 2;
                        continue;
                    }
                    if c == '\'' {
                        if chars.get(i + 1) == Some(&'\'') {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
                code.push('?');
                saw_code = true;
            }
            '"' => {
                i += 1;
                loop {
                    let Some(&c) = chars.get(i) else {
                        return Err(InputError::Unterminated("quoted identifier"));
                    };
                    if c == '"' {
                        if chars.get(i + 1) == Some(&'"') {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
                code.push('"');
                code.push('?');
                code.push('"');
                saw_code = true;
            }
            '$' => {
                let tag_end = chars[i + 1..]
                    .iter()
                    .position(|&c| !(c.is_alphanumeric() || c == '_'))
                    .map(|p| i + 1 + p);
                match tag_end {
                    Some(end)
                        if chars[end] == '$'
                            && (end == i + 1 || !chars[i + 1].is_ascii_digit()) =>
                    {
                        let tag: String = chars[i..=end].iter().collect();
                        let rest: String = chars[end + 1..].iter().collect();
                        let Some(close) = rest.find(&tag) else {
                            return Err(InputError::Unterminated("dollar quote"));
                        };
                        i = end + 1 + rest[..close].chars().count() + tag.chars().count();
                        code.push('?');
                        saw_code = true;
                    }
                    _ => {
                        code.push('$');
                        i += 1;
                        saw_code = true;
                    }
                }
            }
            ';' => {
                let rest_is_blank = chars[i + 1..].iter().all(|c| c.is_whitespace());
                if saw_code && !rest_is_blank {
                    statements += 1;
                }
                saw_code = false;
                code.push(';');
                i += 1;
            }
            _ => {
                if !c.is_whitespace() {
                    saw_code = true;
                }
                code.push(c);
                i += 1;
            }
        }
    }
    Ok((code, statements))
}

fn classify(code: &str) -> Kind {
    let words: Vec<String> = code
        .split(|c: char| c.is_whitespace() || c == '(')
        .filter(|w| !w.is_empty())
        .take(4)
        .map(|w| w.to_ascii_uppercase())
        .collect();
    let first = words.first().map(String::as_str).unwrap_or("");
    let second = words.get(1).map(String::as_str).unwrap_or("");
    let third = words.get(2).map(String::as_str).unwrap_or("");
    match first {
        "SELECT" | "WITH" | "TABLE" | "VALUES" => Kind::Select,
        "EXPLAIN" => Kind::Explain,
        "SHOW" => Kind::Show,
        "VACUUM" => Kind::Vacuum,
        "ANALYZE" | "ANALYSE" => Kind::Analyze,
        "REINDEX" => Kind::Reindex,
        "CLUSTER" => Kind::Cluster,
        "CREATE" if second == "INDEX" || (second == "UNIQUE" && third == "INDEX") => {
            Kind::CreateIndex {
                concurrently: words.iter().any(|w| w == "CONCURRENTLY"),
            }
        }
        "ALTER" => Kind::Alter {
            system: second == "SYSTEM",
        },
        "REFRESH" => Kind::RefreshView,
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" => Kind::Dml(first.to_string()),
        "DROP" => Kind::Drop,
        "TRUNCATE" => Kind::Truncate,
        "BEGIN" | "START" | "COMMIT" | "END" | "ROLLBACK" | "ABORT" | "SAVEPOINT" | "RELEASE"
        | "SET" | "RESET" | "COPY" | "LISTEN" | "UNLISTEN" | "NOTIFY" | "PREPARE" | "EXECUTE"
        | "DEALLOCATE" | "DISCARD" | "LOAD" | "\\" => Kind::Refused(first.to_string()),
        other => Kind::Other(other.chars().take(20).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(sql: &str) -> Kind {
        Statement::parse(sql).unwrap().kind().clone()
    }

    #[test]
    fn statements_are_classified_by_their_first_words() {
        assert_eq!(kind("select 1"), Kind::Select);
        assert_eq!(
            kind(" -- c\n WITH x AS (select 1) select * from x;"),
            Kind::Select
        );
        assert_eq!(kind("(select 1)"), Kind::Select);
        assert_eq!(kind("TABLE demo.orders"), Kind::Select);
        assert_eq!(
            kind("explain (analyze, format json) select 1"),
            Kind::Explain
        );
        assert_eq!(kind("show work_mem"), Kind::Show);
        assert_eq!(kind("VACUUM (VERBOSE) demo.orders"), Kind::Vacuum);
        assert_eq!(kind("analyse demo.orders"), Kind::Analyze);
        assert_eq!(kind("REINDEX INDEX CONCURRENTLY demo.idx"), Kind::Reindex);
        assert_eq!(
            kind("create index concurrently idx on demo.orders (id)"),
            Kind::CreateIndex { concurrently: true }
        );
        assert_eq!(
            kind("create unique index idx on demo.orders (id)"),
            Kind::CreateIndex {
                concurrently: false
            }
        );
        assert_eq!(
            kind("alter table demo.orders add column x int"),
            Kind::Alter { system: false }
        );
        assert_eq!(
            kind("ALTER SYSTEM SET work_mem = '64MB'"),
            Kind::Alter { system: true }
        );
        assert_eq!(
            kind("refresh materialized view concurrently demo.mv"),
            Kind::RefreshView
        );
        assert_eq!(kind("cluster demo.orders using idx"), Kind::Cluster);
        assert_eq!(
            kind("update demo.orders set x = 1"),
            Kind::Dml("UPDATE".into())
        );
        assert_eq!(kind("drop table demo.orders"), Kind::Drop);
        assert_eq!(kind("truncate demo.orders"), Kind::Truncate);
        assert_eq!(
            kind("create table t (id int)"),
            Kind::Other("CREATE".into())
        );
        assert_eq!(kind("begin"), Kind::Refused("BEGIN".into()));
        assert_eq!(kind("set role admin"), Kind::Refused("SET".into()));
        assert_eq!(kind("copy t to program 'sh'"), Kind::Refused("COPY".into()));
        assert_eq!(
            Statement::parse(";;").unwrap_err(),
            InputError::Empty("sql")
        );
        assert_eq!(Statement::parse("select 1").unwrap().text(), "select 1");
    }

    #[test]
    fn strings_comments_and_dollar_quotes_hide_semicolons_and_names() {
        for ok in [
            "select 'a;b', \"c;d\", $$e;f$$, $fn$g;h$fn$ -- ; tail\n /* ; /* nested ; */ */ from t;",
            "select 'it''s'",
            "select E'a\\'b;c'",
            "select 'pg_terminate_backend(1)'",
            "select $1 from t where x = $2",
        ] {
            assert!(Statement::parse(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            Statement::parse("select 1; select 2").unwrap_err(),
            InputError::MultipleStatements(2)
        );
        assert_eq!(
            Statement::parse("select 1;;select 2").unwrap_err(),
            InputError::MultipleStatements(2)
        );
        assert_eq!(
            Statement::parse("select 'open").unwrap_err(),
            InputError::Unterminated("string")
        );
        assert_eq!(
            Statement::parse("select \"open").unwrap_err(),
            InputError::Unterminated("quoted identifier")
        );
        assert_eq!(
            Statement::parse("select $$open").unwrap_err(),
            InputError::Unterminated("dollar quote")
        );
        assert_eq!(
            Statement::parse("select 1 /* open").unwrap_err(),
            InputError::Unterminated("comment")
        );
    }

    #[test]
    fn reads_refuse_server_functions_and_writes_follow_the_level() {
        assert!(
            Statement::parse("select pid from pg_stat_activity")
                .unwrap()
                .require_read()
                .is_ok()
        );
        assert_eq!(
            Statement::parse("select pg_terminate_backend(42)")
                .unwrap()
                .require_read()
                .unwrap_err(),
            InputError::DeniedFunction("pg_terminate_backend".into())
        );
        assert_eq!(
            Statement::parse("SELECT PG_READ_FILE ('x')")
                .unwrap()
                .require_read()
                .unwrap_err(),
            InputError::DeniedFunction("pg_read_file".into())
        );
        assert!(
            Statement::parse("select my_pg_read_file(1)")
                .unwrap()
                .require_read()
                .is_ok()
        );
        assert!(
            Statement::parse("select pg_read_file_count from t")
                .unwrap()
                .require_read()
                .is_ok()
        );
        assert!(matches!(
            Statement::parse("vacuum t")
                .unwrap()
                .require_read()
                .unwrap_err(),
            InputError::NotARead(_)
        ));

        let vacuum = Statement::parse("vacuum t").unwrap();
        assert_eq!(
            vacuum.require_write(WriteLevel::Off).unwrap_err(),
            InputError::WritesDisabled
        );
        assert!(vacuum.require_write(WriteLevel::Maintenance).is_ok());
        assert!(vacuum.kind().outside_transaction());
        let update = Statement::parse("update t set x = 1").unwrap();
        assert!(matches!(
            update.require_write(WriteLevel::Maintenance).unwrap_err(),
            InputError::NeedsAll { .. }
        ));
        assert!(update.require_write(WriteLevel::All).is_ok());
        assert!(!update.kind().outside_transaction());
        let plain_index = Statement::parse("create index i on t (x)").unwrap();
        assert_eq!(
            plain_index
                .require_write(WriteLevel::Maintenance)
                .unwrap_err(),
            InputError::IndexNotConcurrent
        );
        assert!(plain_index.require_write(WriteLevel::All).is_ok());
        for sql in [
            "alter table demo.orders drop column note",
            "alter role r superuser",
            "alter user u password 'x'",
            "alter database d set work_mem = '1MB'",
            "alter default privileges in schema s grant select on tables to r",
        ] {
            let alter = Statement::parse(sql).unwrap();
            assert!(
                matches!(
                    alter.require_write(WriteLevel::Maintenance).unwrap_err(),
                    InputError::NeedsAll { .. }
                ),
                "{sql}"
            );
            assert!(alter.require_write(WriteLevel::All).is_ok(), "{sql}");
        }
        for sql in [
            "alter table demo.orders add column c int",
            "alter index demo.i rename to j",
            "alter sequence s restart",
            "alter materialized view mv set schema demo",
            "alter table t set (autovacuum_vacuum_scale_factor = 0.05)",
        ] {
            assert!(
                Statement::parse(sql)
                    .unwrap()
                    .require_write(WriteLevel::Maintenance)
                    .is_ok(),
                "{sql}"
            );
        }
        assert!(
            Statement::parse("alter table t drop_marker rename to u")
                .unwrap()
                .require_write(WriteLevel::Maintenance)
                .is_ok(),
            "a word that only starts with DROP"
        );
        let system = Statement::parse("alter system set x = 1").unwrap();
        assert!(matches!(
            system.require_write(WriteLevel::Maintenance).unwrap_err(),
            InputError::NeedsAll { .. }
        ));
        assert!(matches!(
            Statement::parse("select 1")
                .unwrap()
                .require_write(WriteLevel::All)
                .unwrap_err(),
            InputError::IsARead(_)
        ));
        assert!(matches!(
            Statement::parse("commit")
                .unwrap()
                .require_write(WriteLevel::All)
                .unwrap_err(),
            InputError::Invalid { .. }
        ));
        assert_eq!(Kind::Refused("SET".into()).level(), None);
        assert_eq!(Kind::Other("GRANT".into()).level(), Some(WriteLevel::All));
        assert_eq!(Kind::Other("GRANT".into()).label(), "GRANT");
    }

    #[test]
    fn the_target_relation_is_found_for_previews() {
        let target = |sql: &str| Statement::parse(sql).unwrap().target();
        assert_eq!(
            target("vacuum (verbose, analyze) demo.orders").as_deref(),
            Some("demo.orders")
        );
        assert_eq!(
            target("analyze demo.orders (id)").as_deref(),
            Some("demo.orders")
        );
        assert_eq!(
            target("reindex table concurrently demo.orders").as_deref(),
            Some("demo.orders")
        );
        assert_eq!(
            target("create index concurrently if not exists i on demo.orders using btree (id)")
                .as_deref(),
            Some("demo.orders")
        );
        assert_eq!(
            target("alter table only demo.orders add column x int").as_deref(),
            Some("demo.orders")
        );
        assert_eq!(
            target("insert into demo.orders (id) values (1)").as_deref(),
            Some("demo.orders")
        );
        assert_eq!(
            target("update demo.orders set x = 1").as_deref(),
            Some("demo.orders")
        );
        assert_eq!(
            target("delete from \"demo\".\"orders\" where id = 1").as_deref(),
            Some("?.?")
        );
        assert_eq!(target("truncate demo.orders"), Some("demo.orders".into()));
        assert_eq!(
            target("drop table if exists demo.orders").as_deref(),
            Some("demo.orders")
        );
        assert_eq!(target("select 1"), None);
        assert_eq!(target("vacuum"), None);
    }
}
