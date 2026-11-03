use super::InputError;

/// The parameters `config_set` may change: the memory and latency knobs
/// the DBA bot's mission names. Anything that touches files, the network,
/// persistence or authentication stays with a person.
pub const CONFIG_SET_ALLOWED: [&str; 14] = [
    "maxmemory",
    "maxmemory-policy",
    "maxmemory-samples",
    "timeout",
    "tcp-keepalive",
    "slowlog-log-slower-than",
    "slowlog-max-len",
    "latency-monitor-threshold",
    "maxclients",
    "lazyfree-lazy-eviction",
    "lazyfree-lazy-expire",
    "lazyfree-lazy-server-del",
    "lazyfree-lazy-user-del",
    "notify-keyspace-events",
];

const POLICIES: [&str; 8] = [
    "noeviction",
    "allkeys-lru",
    "allkeys-lfu",
    "volatile-lru",
    "volatile-lfu",
    "allkeys-random",
    "volatile-random",
    "volatile-ttl",
];

const ALLOWED_TEXT: &str = "maxmemory, maxmemory-policy, maxmemory-samples, timeout, tcp-keepalive, slowlog-log-slower-than, slowlog-max-len, latency-monitor-threshold, maxclients, lazyfree-lazy-*, notify-keyspace-events";

/// The parameter in its canonical spelling and the value checked for the
/// shape it has.
pub fn config_set_check(parameter: &str, value: &str) -> Result<(String, String), InputError> {
    let parameter = parameter.trim().to_ascii_lowercase();
    if !CONFIG_SET_ALLOWED.contains(&parameter.as_str()) {
        return Err(InputError::ParameterNotAllowed(
            parameter.chars().take(60).collect(),
            ALLOWED_TEXT,
        ));
    }
    let value = value.trim();
    let invalid = |rule: &'static str| InputError::Invalid {
        what: "value",
        value: value.chars().take(60).collect(),
        rule,
    };
    let ok = match parameter.as_str() {
        "maxmemory" => {
            let lower = value.to_ascii_lowercase();
            let digits = lower.trim_end_matches(|c: char| c.is_ascii_alphabetic());
            let unit = &lower[digits.len()..];
            !digits.is_empty()
                && digits.chars().all(|c| c.is_ascii_digit())
                && matches!(unit, "" | "b" | "k" | "kb" | "m" | "mb" | "g" | "gb")
        }
        "maxmemory-policy" => POLICIES.contains(&value.to_ascii_lowercase().as_str()),
        "maxmemory-samples"
        | "timeout"
        | "tcp-keepalive"
        | "slowlog-log-slower-than"
        | "slowlog-max-len"
        | "latency-monitor-threshold"
        | "maxclients" => value.parse::<i64>().is_ok_and(|n| n >= -1),
        p if p.starts_with("lazyfree-lazy-") => {
            matches!(value.to_ascii_lowercase().as_str(), "yes" | "no")
        }
        "notify-keyspace-events" => {
            value.len() <= 20 && value.chars().all(|c| "KEg$lshzxeAtmdn".contains(c))
        }
        _ => false,
    };
    if !ok {
        return Err(invalid(match parameter.as_str() {
            "maxmemory" => "bytes with an optional unit, such as 512mb or 2gb",
            "maxmemory-policy" => {
                "one of noeviction, allkeys-lru, allkeys-lfu, volatile-lru, volatile-lfu, allkeys-random, volatile-random, volatile-ttl"
            }
            "notify-keyspace-events" => "the class letters, such as Ex or Kg$",
            p if p.starts_with("lazyfree") => "yes or no",
            _ => "an integer",
        }));
    }
    Ok((parameter, value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_listed_parameters_pass_and_their_values_have_a_shape() {
        assert_eq!(
            config_set_check("MAXMEMORY", " 512mb ").unwrap(),
            ("maxmemory".into(), "512mb".into())
        );
        assert!(config_set_check("maxmemory", "1073741824").is_ok());
        assert!(config_set_check("maxmemory", "lots").is_err());
        assert!(config_set_check("maxmemory", "512tb").is_err());
        assert_eq!(
            config_set_check("maxmemory-policy", "allkeys-lru")
                .unwrap()
                .1,
            "allkeys-lru"
        );
        assert!(config_set_check("maxmemory-policy", "random").is_err());
        assert!(config_set_check("slowlog-log-slower-than", "10000").is_ok());
        assert!(config_set_check("timeout", "-1").is_ok());
        assert!(config_set_check("timeout", "soon").is_err());
        assert!(config_set_check("lazyfree-lazy-expire", "yes").is_ok());
        assert!(config_set_check("lazyfree-lazy-expire", "maybe").is_err());
        assert!(config_set_check("notify-keyspace-events", "Ex").is_ok());
        assert!(config_set_check("notify-keyspace-events", "Q").is_err());
        for forbidden in [
            "dir",
            "dbfilename",
            "bind",
            "port",
            "protected-mode",
            "appendonly",
            "save",
            "aclfile",
            "requirepass",
            "masteruser",
            "masterauth",
            "rename-command",
        ] {
            assert!(
                matches!(
                    config_set_check(forbidden, "x").unwrap_err(),
                    InputError::ParameterNotAllowed(..)
                ),
                "{forbidden}"
            );
        }
        assert_eq!(CONFIG_SET_ALLOWED.len(), 14);
    }
}
