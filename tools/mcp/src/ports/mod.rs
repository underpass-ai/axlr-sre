#[cfg(test)]
mod fake_tool_host;
mod tool_host;

#[cfg(test)]
pub(crate) use fake_tool_host::FakeToolHost;
pub use tool_host::ToolHost;
