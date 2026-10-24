//! DERIVED FROM CODEX — keep in step with the vendored upstream file:
//!   vendor/codex-sandbox/upstream/codex-rs/mxc-sandbox/src/availability.rs (openai/codex @ 970111f6cb62cc02fe36a82ea7d71b37a2889c97)
//! Licensed under the Apache License 2.0 (OpenAI Codex), see NOTICE.
//! Changes from upstream: the `ContainerPolicy` default and the probe body are unchanged in substance; written with
//! Agenium's imports (mxc-sdk 1.0.0 from crates.io instead of the git revision Codex pins).

use std::sync::OnceLock;

use flatbuffers::FlatBufferBuilder;
use mxc_sdk::mxc_common::models::ContainerPolicy;
use mxc_sdk::mxc_common::ui_policy::resolve_ui_restrictions;
use mxc_sdk::process_container_common::job_object::to_job_object_uilimit_mask;
use mxc_sdk::process_container_common::{
    ProcessSecurityEnvironmentApi, SecurityEnvironmentStartupInfo,
    PROCESS_SECURITY_ENVIRONMENT_FLAG_NONE,
};
use mxc_sdk::process_security_environment_spec::process_security_environment_layout::{
    finish_process_security_environment_buffer, EndpointPolicyT, FilterAction, NetworkPolicyT,
    ProcessSecurityEnvironmentT, SchemaVersionT,
};

/// Whether a native MXC process security environment can be created here (probe without launching anything;
/// cached). Older AppContainer fallbacks of MXC do not count.
pub fn available() -> bool {
    static USABLE: OnceLock<bool> = OnceLock::new();
    *USABLE.get_or_init(|| {
        let policy = ContainerPolicy::default();
        let mut egress = EndpointPolicyT::default();
        egress.default_action = FilterAction::deny;
        let mut network = NetworkPolicyT::default();
        network.egress = Some(Box::new(egress));
        let mut spec = ProcessSecurityEnvironmentT::default();
        spec.version = SchemaVersionT { major: 1, minor: 0 };
        spec.disallow_win32k_system_calls = policy.ui.disable;
        spec.ui_restrictions = u64::from(to_job_object_uilimit_mask(&resolve_ui_restrictions(
            &policy.ui,
            &policy.base_process_ui,
        )));
        spec.network_policy = Some(Box::new(network));
        let mut builder = FlatBufferBuilder::with_capacity(1024);
        let packed = spec.pack(&mut builder);
        finish_process_security_environment_buffer(&mut builder, packed);
        ProcessSecurityEnvironmentApi::load()
            .and_then(|api| {
                api.create(
                    builder.finished_data(),
                    PROCESS_SECURITY_ENVIRONMENT_FLAG_NONE,
                )
            })
            .and_then(|env| {
                let info = SecurityEnvironmentStartupInfo::new(Default::default(), env.raw(), &[]);
                env.close();
                info.map(drop)
            })
            .is_ok()
    })
}
