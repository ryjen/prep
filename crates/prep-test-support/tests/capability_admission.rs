use prep_core::{
    PluginAdmissionPolicy, PluginProbeError, PluginProcessPolicy, probe_build_system_with_policies,
    probe_build_system_with_policy,
};
use prep_protocol::PluginCapability;
use std::path::Path;

#[test]
fn declared_network_capability_is_denied_by_default() {
    let plugin = Path::new(env!("CARGO_BIN_EXE_prep-network-capability-plugin"));
    let error = probe_build_system_with_policy(plugin, PluginProcessPolicy::default())
        .expect_err("default admission policy must deny network");

    assert!(matches!(
        error,
        PluginProbeError::PolicyDenied {
            capability: PluginCapability::Network
        }
    ));
}

#[test]
fn explicitly_allowed_network_capability_reaches_the_operation() {
    let plugin = Path::new(env!("CARGO_BIN_EXE_prep-network-capability-plugin"));
    let admission = PluginAdmissionPolicy::allow([PluginCapability::Network]);
    let probe = probe_build_system_with_policies(
        plugin,
        PluginProcessPolicy::default(),
        admission,
    )
    .expect("explicitly allowed capability should admit the operation");

    assert_eq!(probe.name, "declare-network");
    assert!(probe.supported);
}
