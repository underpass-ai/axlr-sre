/// How a tool's JSON answer is written into the MCP text block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Indented, for a person or a model reading it.
    #[default]
    Pretty,
    /// One line, for a host that counts tokens.
    Compact,
}
