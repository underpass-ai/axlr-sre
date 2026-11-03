use super::{InputError, tokens};

const MAX_REPLICAS_CEILING: u32 = 1000;

/// What this server may touch: the namespaces, whether it writes, how far
/// it scales. Checked before any `kubectl` runs; RBAC checks again after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    namespaces: Vec<String>,
    writes: bool,
    max_replicas: u32,
}

impl Policy {
    /// An empty allowlist means any valid namespace, named on every call.
    pub fn new(
        namespaces: Vec<String>,
        writes: bool,
        max_replicas: u32,
    ) -> Result<Self, InputError> {
        let namespaces = namespaces
            .iter()
            .map(|namespace| tokens::namespace(namespace))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            namespaces,
            writes,
            max_replicas: max_replicas.clamp(1, MAX_REPLICAS_CEILING),
        })
    }

    #[must_use]
    pub fn read_only() -> Self {
        Self {
            namespaces: Vec::new(),
            writes: false,
            max_replicas: 10,
        }
    }

    /// The namespace of a call: the one sent, if the allowlist has it, or the
    /// first of the allowlist when none is sent.
    pub fn namespace(&self, requested: Option<&str>) -> Result<String, InputError> {
        match requested {
            Some(requested) => {
                let namespace = tokens::namespace(requested)?;
                if self.namespaces.is_empty() || self.namespaces.contains(&namespace) {
                    Ok(namespace)
                } else {
                    Err(InputError::NamespaceNotAllowed {
                        namespace,
                        allowed: self.namespaces.join(", "),
                    })
                }
            }
            None => self
                .namespaces
                .first()
                .cloned()
                .ok_or(InputError::NamespaceRequired),
        }
    }

    #[must_use]
    pub fn namespaces(&self) -> &[String] {
        &self.namespaces
    }

    #[must_use]
    pub fn writes(&self) -> bool {
        self.writes
    }

    pub fn require_writes(&self) -> Result<(), InputError> {
        if self.writes {
            Ok(())
        } else {
            Err(InputError::WritesDisabled)
        }
    }

    #[must_use]
    pub fn max_replicas(&self) -> u32 {
        self.max_replicas
    }

    /// A replica count inside the cap; zero is allowed, it is how a workload is stopped.
    pub fn replicas(&self, requested: i64) -> Result<u32, InputError> {
        u32::try_from(requested)
            .ok()
            .filter(|replicas| *replicas <= self.max_replicas)
            .ok_or(InputError::OutOfRange {
                what: "replicas",
                min: 0,
                max: i64::from(self.max_replicas),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_allowlist_bounds_the_namespace_and_gives_the_default() {
        let policy = Policy::new(vec!["prod".into(), "staging".into()], false, 5).unwrap();
        assert_eq!(policy.namespace(None).unwrap(), "prod");
        assert_eq!(policy.namespace(Some("staging")).unwrap(), "staging");
        assert_eq!(
            policy.namespace(Some("other")),
            Err(InputError::NamespaceNotAllowed {
                namespace: "other".into(),
                allowed: "prod, staging".into()
            })
        );
        assert!(matches!(
            policy.namespace(Some("Bad")),
            Err(InputError::InvalidNamespace(_))
        ));
        assert_eq!(policy.namespaces(), ["prod", "staging"]);
    }

    #[test]
    fn without_an_allowlist_every_call_names_its_namespace() {
        let policy = Policy::read_only();
        assert_eq!(policy.namespace(None), Err(InputError::NamespaceRequired));
        assert_eq!(policy.namespace(Some("any")).unwrap(), "any");
        assert!(!policy.writes());
        assert_eq!(policy.require_writes(), Err(InputError::WritesDisabled));
    }

    #[test]
    fn replicas_stay_inside_the_cap() {
        let policy = Policy::new(vec![], true, 3).unwrap();
        assert!(policy.require_writes().is_ok());
        assert_eq!(policy.replicas(0).unwrap(), 0);
        assert_eq!(policy.replicas(3).unwrap(), 3);
        assert!(policy.replicas(4).is_err());
        assert!(policy.replicas(-1).is_err());
        assert_eq!(policy.max_replicas(), 3);
    }

    #[test]
    fn the_cap_itself_is_clamped_and_the_namespaces_validated() {
        assert_eq!(Policy::new(vec![], false, 0).unwrap().max_replicas(), 1);
        assert_eq!(
            Policy::new(vec![], false, 5000).unwrap().max_replicas(),
            1000
        );
        assert!(Policy::new(vec!["Bad".into()], false, 1).is_err());
    }
}
