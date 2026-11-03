//! SQS and SNS for a bot that fixes queues, through the `aws` CLI: the
//! queues of a region with their depth, redrive policy and sources, a look
//! at a queue's messages without consuming them, the topics and their
//! subscriptions; and, with `MESSAGING_WRITES`, send, publish, the native
//! redrive of a dead-letter queue, its cancel, and a drain that saves every
//! message to disk before deleting it. Every write previews unless
//! `confirm` is true. The runner, the regions, the failure translation and
//! the metric series come from the `aws-mcp` crate.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;
