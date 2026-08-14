mod graph;
mod identity;
mod plugin;

pub use graph::{DependencyGraph, GraphError};
pub use identity::{
    BuildInput, DependencyResult, IdentityError, PluginBuildIdentity, ResultId, ToolIdentity,
};
pub use plugin::{
    PluginDiagnostics, PluginPhase, PluginProbe, PluginProbeError, PluginProcessPolicy,
    probe_build_system, probe_build_system_with_policy,
};
