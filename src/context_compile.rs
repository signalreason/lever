use std::path::PathBuf;

pub const DEFAULT_CONTEXT_TOKEN_BUDGET: u64 = 8_000;
pub const DEFAULT_ASSEMBLY_PATH: &str = "assembly";
pub const DEFAULT_CONTEXT_EXCLUDE_GLOBS: &[&str] = &[".git/**", ".ralph/**"];

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum ContextFailurePolicy {
    #[default]
    BestEffort,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCompileConfig {
    pub enabled: bool,
    pub policy: ContextFailurePolicy,
    pub token_budget: u64,
    pub assembly_path: PathBuf,
    pub exclude_globs: Vec<String>,
    pub exclude_runtime_globs: Vec<String>,
}

impl Default for ContextCompileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            policy: ContextFailurePolicy::BestEffort,
            token_budget: DEFAULT_CONTEXT_TOKEN_BUDGET,
            assembly_path: PathBuf::from(DEFAULT_ASSEMBLY_PATH),
            exclude_globs: DEFAULT_CONTEXT_EXCLUDE_GLOBS
                .iter()
                .copied()
                .map(str::to_string)
                .collect(),
            exclude_runtime_globs: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_compile_config_defaults() {
        let config = ContextCompileConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.policy, ContextFailurePolicy::BestEffort);
        assert_eq!(config.token_budget, DEFAULT_CONTEXT_TOKEN_BUDGET);
        assert_eq!(config.assembly_path, PathBuf::from(DEFAULT_ASSEMBLY_PATH));
        assert_eq!(
            config.exclude_globs,
            DEFAULT_CONTEXT_EXCLUDE_GLOBS
                .iter()
                .copied()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
        assert!(config.exclude_runtime_globs.is_empty());
    }
}
