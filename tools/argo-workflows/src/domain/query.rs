use super::QueryError;

type Result<T> = std::result::Result<T, QueryError>;

/// The three kinds a listing can page through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Workflows,
    WorkflowTemplates,
    CronWorkflows,
}

/// The three kinds a detail can fetch whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Workflow,
    WorkflowTemplate,
    CronWorkflow,
}

/// A validated, read-only question to Argo Server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgoQuery {
    List {
        kind: ListKind,
        namespace: String,
        limit: u16,
        continuation: Option<String>,
    },
    Get {
        kind: ResourceKind,
        namespace: String,
        name: String,
    },
    Logs {
        namespace: String,
        name: String,
        pod_name: Option<String>,
        tail_lines: u16,
    },
    Version,
}

impl ArgoQuery {
    pub fn list(
        kind: ListKind,
        namespace: String,
        limit: u16,
        continuation: Option<String>,
    ) -> Result<Self> {
        validate_name(&namespace, "namespace")?;
        if !(1..=100).contains(&limit) {
            return Err(QueryError::InvalidLimit);
        }
        if continuation
            .as_ref()
            .is_some_and(|cursor| cursor.len() > 4096)
        {
            return Err(QueryError::LongCursor);
        }
        Ok(Self::List {
            kind,
            namespace,
            limit,
            continuation,
        })
    }

    pub fn get(kind: ResourceKind, namespace: String, name: String) -> Result<Self> {
        validate_name(&namespace, "namespace")?;
        validate_name(&name, "name")?;
        Ok(Self::Get {
            kind,
            namespace,
            name,
        })
    }

    pub fn logs(
        namespace: String,
        name: String,
        pod_name: Option<String>,
        tail_lines: u16,
    ) -> Result<Self> {
        validate_name(&namespace, "namespace")?;
        validate_name(&name, "name")?;
        if let Some(pod) = &pod_name {
            validate_name(pod, "pod_name")?;
        }
        if !(1..=200).contains(&tail_lines) {
            return Err(QueryError::InvalidTailLines);
        }
        Ok(Self::Logs {
            namespace,
            name,
            pod_name,
            tail_lines,
        })
    }

    /// The API path segments: collection, namespace and, for a detail, the name.
    #[must_use]
    pub fn endpoint(&self) -> (&'static str, Option<&str>, Option<&str>) {
        match self {
            Self::List {
                kind, namespace, ..
            } => (kind.collection(), Some(namespace), None),
            Self::Get {
                kind,
                namespace,
                name,
            } => (kind.collection(), Some(namespace), Some(name)),
            Self::Logs {
                namespace, name, ..
            } => ("workflows", Some(namespace), Some(name)),
            Self::Version => ("version", None, None),
        }
    }

    #[must_use]
    pub fn page(&self) -> Option<(u16, Option<&str>)> {
        match self {
            Self::List {
                limit,
                continuation,
                ..
            } => Some((*limit, continuation.as_deref())),
            _ => None,
        }
    }

    #[must_use]
    pub fn log_options(&self) -> Option<(Option<&str>, u16)> {
        match self {
            Self::Logs {
                pod_name,
                tail_lines,
                ..
            } => Some((pod_name.as_deref(), *tail_lines)),
            _ => None,
        }
    }
}

impl ListKind {
    #[must_use]
    pub fn collection(self) -> &'static str {
        match self {
            Self::Workflows => "workflows",
            Self::WorkflowTemplates => "workflow-templates",
            Self::CronWorkflows => "cron-workflows",
        }
    }
}

impl ResourceKind {
    #[must_use]
    pub fn collection(self) -> &'static str {
        match self {
            Self::Workflow => "workflows",
            Self::WorkflowTemplate => "workflow-templates",
            Self::CronWorkflow => "cron-workflows",
        }
    }
}

fn validate_name(value: &str, field: &'static str) -> Result<()> {
    let ok = !value.is_empty()
        && value.len() <= 253
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        && !value.starts_with(['-', '.'])
        && !value.ends_with(['-', '.']);
    if ok {
        Ok(())
    } else {
        Err(QueryError::InvalidName { field })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_keeps_its_page_and_endpoint() {
        let query = ArgoQuery::list(
            ListKind::CronWorkflows,
            "ci".into(),
            20,
            Some("next".into()),
        )
        .unwrap();
        assert_eq!(query.endpoint(), ("cron-workflows", Some("ci"), None));
        assert_eq!(query.page(), Some((20, Some("next"))));
        assert!(query.log_options().is_none());
    }

    #[test]
    fn a_detail_names_its_resource() {
        let query =
            ArgoQuery::get(ResourceKind::WorkflowTemplate, "ci".into(), "build".into()).unwrap();
        assert_eq!(
            query.endpoint(),
            ("workflow-templates", Some("ci"), Some("build"))
        );
        assert!(query.page().is_none());
    }

    #[test]
    fn logs_carry_their_pod_and_tail() {
        let query =
            ArgoQuery::logs("ci".into(), "run-1".into(), Some("run-1-pod".into()), 50).unwrap();
        assert_eq!(query.endpoint(), ("workflows", Some("ci"), Some("run-1")));
        assert_eq!(query.log_options(), Some((Some("run-1-pod"), 50)));
    }

    #[test]
    fn the_version_has_no_namespace() {
        assert_eq!(ArgoQuery::Version.endpoint(), ("version", None, None));
    }

    #[test]
    fn bad_input_is_refused_before_any_request() {
        assert_eq!(
            ArgoQuery::list(ListKind::Workflows, "Bad".into(), 10, None),
            Err(QueryError::InvalidName { field: "namespace" })
        );
        assert_eq!(
            ArgoQuery::list(ListKind::Workflows, "ci".into(), 0, None),
            Err(QueryError::InvalidLimit)
        );
        assert_eq!(
            ArgoQuery::list(ListKind::Workflows, "ci".into(), 101, None),
            Err(QueryError::InvalidLimit)
        );
        assert_eq!(
            ArgoQuery::list(ListKind::Workflows, "ci".into(), 10, Some("x".repeat(4097))),
            Err(QueryError::LongCursor)
        );
        assert_eq!(
            ArgoQuery::get(ResourceKind::Workflow, "ci".into(), "../etc".into()),
            Err(QueryError::InvalidName { field: "name" })
        );
        assert_eq!(
            ArgoQuery::logs("ci".into(), "run".into(), Some("Pod".into()), 10),
            Err(QueryError::InvalidName { field: "pod_name" })
        );
        assert_eq!(
            ArgoQuery::logs("ci".into(), "run".into(), None, 201),
            Err(QueryError::InvalidTailLines)
        );
        assert_eq!(
            ArgoQuery::logs("ci".into(), "-run".into(), None, 10),
            Err(QueryError::InvalidName { field: "name" })
        );
    }
}
