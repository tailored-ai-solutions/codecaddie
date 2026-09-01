//! Local CodeCaddie core process.

pub mod agent_cli;
pub(crate) mod agent_gateway;
pub mod analyzer;
mod at_rest;
pub mod context_documents;
pub mod export;
pub mod launch_at_login;
pub mod local_state;
pub mod mcp;
pub mod persistence;
#[cfg(test)]
pub(crate) mod privacy_test_support;
pub mod protocol;
pub mod provider;
pub mod provider_repository_mcp;
pub mod reliability;
pub(crate) mod report_integrity;
pub mod repository;
pub mod runtime_channel;
pub mod runtime_controls;
pub mod service;
pub mod storage;
pub mod update;

#[cfg(test)]
mod decision_journey_assurance;
#[cfg(test)]
mod operational_fault_assurance;
#[cfg(test)]
mod product_assurance;
#[cfg(test)]
mod runtime_health_assurance;
