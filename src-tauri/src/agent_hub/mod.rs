//! Pluggable Multi-Agent CLI Router, Storage Context Payload builder,
//! Mandatory Multi-Source Internet Cross-Check Prompt & Response Validator.

pub mod context;
pub mod prompt;
pub mod providers;
pub mod runner;
pub mod validator;

pub use context::{ContextError, ExtensionStat, LockedHandleStatus, StorageContextPayload};
pub use prompt::build_cross_check_system_prompt;
pub use providers::{
    build_opencode_isolated_config, build_opencode_isolated_config_from_contents,
    default_search_dirs_and_exts, discover_all_provider_statuses, discover_available_providers,
    discover_providers_in, resolve_binary_in_dirs, AgentProvider, AgentProviderKind,
    CliInvocationSpec, CustomCliConfig, DiscoveredProvider, InvocationInput, ProviderError,
    ProviderStatus,
};
pub use runner::{
    AgentHubError, AgentHubRunner, CommandResolver, ConnectivityReport, DefaultCommandResolver,
    ExecutionOutput, OverrideCommandResolver, RawChatExecutionOutput, ResolvedCommand, RunnerConfig,
    StreamChunk, StreamSource,
};
pub use validator::{
    extract_assistant_text, extract_external_urls_from_text, parse_and_validate_advisor_response,
    AdvisorResponse, AdvisorVerdict, CrossCheckedSource, ValidationError,
};
