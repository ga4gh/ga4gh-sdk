//! Rust clients for GA4GH APIs: TES, WES, DRS, TRS, Service Info and the
//! Service Registry.
//!
//! Build a [`Configuration`](utils::configuration::Configuration) with a
//! service's API root, or read one from the configuration file, then a client
//! from [`clients`], such as [`TES`](clients::tes::TES). The clients send their
//! requests with a [`Transport`](utils::transport::Transport), which keeps them
//! below the API root and applies the configuration's credentials and time
//! limits.
//!
//! ```no_run
//! use ga4gh_sdk::clients::tes::TES;
//! use ga4gh_sdk::utils::configuration::Configuration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = Configuration::new(url::Url::parse("https://example.org/ga4gh/tes/v1")?);
//! let tes = TES::new(&config).await?;
//! let page = tes.list_tasks(None).await?;
//! println!("{} tasks", page.tasks.len());
//! # Ok(())
//! # }
//! ```
//!
//! The feature `workflow-contracts` adds the module `workflow`: revisioned
//! workflow contracts, which prepare, submit and compare WES runs;
//! preparation and comparison never execute jobs.
#![warn(missing_docs)]

#[allow(unused_imports)]
#[macro_use]
extern crate serde_derive;

pub mod clients;
pub mod compatibility;
pub mod utils;

/// HTTP types used by the public transport API.
pub use reqwest;

// Documented in #270; `workflow::reuse` is checked already (#269)
#[allow(missing_docs)]
#[cfg(feature = "workflow-contracts")]
pub mod workflow;
