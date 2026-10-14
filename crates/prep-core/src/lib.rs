mod archive_policy;
mod archive_tar;
mod git_command;
mod git_run;
mod git_tree;
mod graph;
mod identity;
mod plugin;
mod plugin_input;
mod source;
mod source_load;
mod source_extract;
mod verified_ustar;

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

pub use git_command::{
    isolated_git_command, parse_exact_git_ref, prepare_exact_git_ref_resolution,
    prepare_locked_tree_inspection,
};

pub use archive_tar::inspect_ustar;

pub use verified_ustar::{
    MAX_ARCHIVE_SNAPSHOT_BYTES, VerifiedUstarSnapshot, read_verified_ustar_snapshot,
};

pub use source_load::{SourceLoadError, load_cached_ustar, load_staged_ustar};

pub use git_run::{GitResolutionError, GitResolutionLimits, resolve_git_reference};

pub use source_extract::{SourceExtractionError, extract_verified_ustar};
