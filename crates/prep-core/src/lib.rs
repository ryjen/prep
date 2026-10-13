mod graph;
mod identity;
mod plugin;
mod plugin_input;

pub use graph::{DependencyGraph, GraphError};
pub use identity::{
    BuildInput, DependencyResult, IdentityError, PluginBuildIdentity, ResultId, ToolIdentity,
};
pub use plugin::{
    PluginAdmissionPolicy, PluginDiagnostics, PluginPhase, PluginProbe, PluginProbeError,
    PluginProcessPolicy, prepare_host_planning_request, validate_host_planning_response, probe_build_system,
    probe_build_system_with_policies, probe_build_system_with_policy,
};

pub use plugin_input::{
    AuthorizedBuilderOperation, BuilderRequestInput, PluginEnvironmentPolicy, PluginExecutionRoots,
    PluginInputError, SanitizedPluginEnvironment, authorize_builder_operation,
    required_builder_capabilities,
};
