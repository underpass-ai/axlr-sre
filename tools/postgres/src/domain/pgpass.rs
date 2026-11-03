/// One line of a libpq password file: `host:port:database:user:password`,
/// `*` matching anything in the first four fields, `\:` and `\\` escapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgPass {
    host: String,
    port: String,
    database: String,
    user: String,
    password: String,
}

impl PgPass {
    /// Every well-formed line; comments and short lines are skipped.
    #[must_use]
    pub fn parse(text: &str) -> Vec<Self> {
        text.lines()
            .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .filter_map(|line| {
                let fields = split_fields(line);
                (fields.len() == 5).then(|| Self {
                    host: fields[0].clone(),
                    port: fields[1].clone(),
                    database: fields[2].clone(),
                    user: fields[3].clone(),
                    password: fields[4].clone(),
                })
            })
            .collect()
    }

    fn matches(&self, host: &str, port: u16, database: &str, user: &str) -> bool {
        let same = |pattern: &str, value: &str| pattern == "*" || pattern == value;
        same(&self.host, host)
            && same(&self.port, &port.to_string())
            && same(&self.database, database)
            && same(&self.user, user)
    }
}

fn split_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ':' => fields.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    fields.push(current);
    fields
}

/// The first matching password, as libpq picks it.
#[must_use]
pub fn pgpass_lookup(
    text: &str,
    host: &str,
    port: u16,
    database: &str,
    user: &str,
) -> Option<String> {
    PgPass::parse(text)
        .into_iter()
        .find(|entry| entry.matches(host, port, database, user))
        .map(|entry| entry.password)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# comment\n\ndb.example:5432:orders:app:plain\n*:*:orders:ro:any-host\nweird\\:host:5432:*:*:with\\\\slash\nshort:line\n";

    #[test]
    fn the_first_matching_line_wins_and_escapes_are_read() {
        assert_eq!(
            pgpass_lookup(FILE, "db.example", 5432, "orders", "app").as_deref(),
            Some("plain")
        );
        assert_eq!(
            pgpass_lookup(FILE, "other", 5433, "orders", "ro").as_deref(),
            Some("any-host")
        );
        assert_eq!(
            pgpass_lookup(FILE, "weird:host", 5432, "x", "y").as_deref(),
            Some("with\\slash")
        );
        assert_eq!(
            pgpass_lookup(FILE, "db.example", 5433, "orders", "app"),
            None
        );
        assert_eq!(PgPass::parse(FILE).len(), 3);
    }
}
