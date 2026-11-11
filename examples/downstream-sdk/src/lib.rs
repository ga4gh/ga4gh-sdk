//! An example of a downstream SDK for a TES service with API extensions.
//!
//! It uses only public APIs of `ga4gh-lib` (crate `ga4gh_sdk`) and
//! `ga4gh-cli`, and shows the three ways to build on them:
//!
//! - [`usage`]: a client for an additional endpoint that shares the SDK's
//!   [`Transport`](ga4gh_sdk::utils::transport::Transport)
//! - [`task`]: a model that extends
//!   [`TesTask`](ga4gh_sdk::clients::tes::models::TesTask) with a field
//!   that the TES specification doesn't define
//! - [`cli`]: a CLI that adds a subcommand to the SDK's CLI
//!
//! The extensions themselves (`GET /tasks/{id}/usage` and a `priority` field
//! on tasks) are made up for the example. The CI workflow builds and tests
//! this crate, so that changes to the SDK can't break it unnoticed.

pub mod cli;
pub mod task;
pub mod usage;
