//! Kubernetes through `kubectl`, one tool per verb, for the SRE bots.
//!
//! A read-only server answers `get`, `describe`, `logs`, `events`, the
//! rollout status and history; a server registered with writes enabled also
//! answers `diff`, `apply`, `rollout_restart`, `rollout_undo` and `scale`,
//! every write previewed unless `confirm` is true. The policy is by tool and
//! by namespace, never by glob: each call names its namespace and the server
//! checks it against its allowlist before `kubectl` runs.
//!
//! `domain` validates every token, the manifests and the policy; `ports` is
//! the one thing the use cases need, a runner of `kubectl`; `application`
//! shapes the answers and runs `doctor` and `discover`; `adapters::outbound`
//! spawns the real process and `adapters::inbound` is the catalogue and the
//! dispatch.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;
