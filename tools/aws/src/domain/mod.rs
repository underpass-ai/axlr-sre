mod alarms;
mod errors;
mod failure;
mod invocation;
mod lambda;
mod logs;
mod regions;
mod series;
mod server_config;
mod time_window;
mod tokens;

pub use alarms::alarms;
pub use errors::{ConfigError, InputError};
pub use failure::Failure;
pub use invocation::{
    ALARMS_MAX, EVENTS_MAX, FUNCTIONS_MAX, GROUPS_MAX, HEALTH_METRICS, Invocation, MetricQuery,
    PAGE_MAX,
};
pub use lambda::{function_detail, function_summary, log_group_of, triggers_of};
pub use logs::{log_events, log_groups};
pub use regions::Regions;
pub use series::{Series, series_of};
pub use server_config::ServerConfig;
pub use time_window::{TimeWindow, instant};
pub use tokens::{
    alarm_prefix, dimension, filter_pattern, function_name, log_group, metric_name, namespace,
    period, region, statistic,
};
