//! Application-owned execution contract shared by every provider adapter.
//!
//! Provider CLIs have different flags and output envelopes, but callers see
//! one versioned lifecycle: bounded output, one selected adapter, typed safe
//! failures, timeout, cancellation by dropping the run future, and no fallback
//! to a different provider.

use super::ProviderKind;
use std::{error::Error, fmt, time::Duration};

pub(super) const VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureCode {
    InvalidContract,
    StartFailed,
    IoFailed,
    MalformedResult,
    TimedOut,
    ProviderFailed,
}

impl FailureCode {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidContract => "invalid_contract",
            Self::StartFailed => "start_failed",
            Self::IoFailed => "io_failed",
            Self::MalformedResult => "malformed_result",
            Self::TimedOut => "timed_out",
            Self::ProviderFailed => "provider_failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FallbackPolicy {
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ExecutionContract {
    pub(super) version: u8,
    pub(super) provider: ProviderKind,
    pub(super) timeout: Duration,
    pub(super) output_limit_bytes: usize,
    pub(super) fallback: FallbackPolicy,
}

impl ExecutionContract {
    pub(super) const fn new(
        provider: ProviderKind,
        timeout: Duration,
        output_limit_bytes: usize,
    ) -> Self {
        Self {
            version: VERSION,
            provider,
            timeout,
            output_limit_bytes,
            fallback: FallbackPolicy::Forbidden,
        }
    }

    pub(super) fn failure(self, code: FailureCode, message: impl Into<String>) -> anyhow::Error {
        ProviderContractError {
            version: self.version,
            provider: self.provider,
            code,
            safe_message: message.into(),
        }
        .into()
    }
}

#[derive(Debug)]
pub(crate) struct ProviderContractError {
    pub(super) version: u8,
    pub(super) provider: ProviderKind,
    pub(super) code: FailureCode,
    safe_message: String,
}

impl fmt::Display for ProviderContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "provider contract v{} {} {}: {}",
            self.version,
            self.provider.executable(),
            self.code.as_str(),
            self.safe_message
        )
    }
}

impl Error for ProviderContractError {}
