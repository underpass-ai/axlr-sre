use thiserror::Error;

/// What is wrong with a call's arguments, said before `kubectl` runs.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error("invalid namespace `{0}`: a DNS label, lowercase, 63 characters at most")]
    InvalidNamespace(String),
    #[error(
        "invalid name `{0}`: a Kubernetes name, lowercase, dots and hyphens, 253 characters at most"
    )]
    InvalidName(String),
    #[error("invalid resource kind `{0}`: lowercase, as kubectl names it (pods, deployments.apps)")]
    InvalidKind(String),
    #[error(
        "secrets are never read by this server: their names appear in the pod's events and describe"
    )]
    SecretsRefused,
    #[error("invalid label selector `{0}`")]
    InvalidSelector(String),
    #[error("invalid duration `{0}`: a number and s, m or h, such as 30s or 5m")]
    InvalidDuration(String),
    #[error("{what} must be between {min} and {max}")]
    OutOfRange {
        what: &'static str,
        min: i64,
        max: i64,
    },
    #[error("namespace `{namespace}` is outside this server's allowlist ({allowed})")]
    NamespaceNotAllowed { namespace: String, allowed: String },
    #[error("namespace is required: this server has no default namespace")]
    NamespaceRequired,
    #[error("this server is read-only")]
    WritesDisabled,
    #[error("the manifest is empty")]
    EmptyManifest,
    #[error("the manifest is too large: {0} bytes, 262144 at most")]
    ManifestTooLarge(usize),
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("kind {0} is never written by this server")]
    ForbiddenKind(String),
    #[error("{kind} `{name}` names namespace `{namespace}`, not the one of this call")]
    ForeignNamespace {
        kind: String,
        name: String,
        namespace: String,
    },
    #[error("unknown {what} `{value}`: one of {allowed}")]
    UnknownChoice {
        what: &'static str,
        value: String,
        allowed: &'static str,
    },
}

/// What is wrong with the environment the server started with.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("KUBE_NAMESPACES has an invalid namespace: {0}")]
    InvalidNamespaces(String),
    #[error("KUBE_WRITES must be true or false, not `{0}`")]
    InvalidWrites(String),
    #[error("KUBE_MAX_REPLICAS must be a number from 1 to 1000, not `{0}`")]
    InvalidMaxReplicas(String),
    #[error("KUBE_CONTEXT `{0}` is not a context name")]
    InvalidContext(String),
    #[error("KUBECTL must be a path to the kubectl binary")]
    InvalidKubectl,
}
