mod archive_policy;
mod git_command;
mod git_tree;
mod graph;
mod identity;
mod plugin;
mod plugin_input;
mod source;

pub use graph::{DependencyGraph, GraphError};
pub use identity::{
    BuildInput, DependencyResult, IdentityError, PluginBuildIdentity, ResultId, ToolIdentity,
};
pub use plugin::{
    PluginAdmissionPolicy, PluginDiagnostics, PluginPhase, PluginProbe, PluginProbeError,
    PluginProcessPolicy, prepare_host_planning_request, probe_build_system,
    probe_build_system_with_policies, probe_build_system_with_policy,
    validate_host_planning_response,
};

pub use plugin_input::{
    AuthorizedBuilderOperation, BuilderRequestInput, PluginEnvironmentPolicy, PluginExecutionRoots,
    PluginInputError, SanitizedPluginEnvironment, authorize_builder_operation,
    required_builder_capabilities,
};

pub use source::{
    SourceVerificationError, validate_archive_entry_path, verify_archive_bytes, verify_git_head,
};

pub use archive_policy::{
    ArchiveEntry, ArchiveEntryKind, ArchiveLimits, validate_archive_inventory,
};

pub use git_tree::validate_git_tree_listing;

pub use git_command::{isolated_git_command, prepare_locked_tree_inspection};
