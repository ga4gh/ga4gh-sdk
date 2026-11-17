//! The deprecated client, `Discovered`, and `Transport` fields stay public
//! until the breaking release (see `docs/state_ownership.md`). These are the
//! assignments of the `compile_fail` documentation tests on `TES`, `WES`,
//! `DRS`, `Discovered`, and `Transport`: they compile here, so those tests fail
//! only because `#![deny(deprecated)]` rejects them. Remove this file when the
//! fields become private.
#![allow(deprecated)]

use ga4gh_sdk::{
    clients::{capabilities::Discovered, drs::DRS, tes::TES, wes::WES},
    utils::{configuration::Configuration, transport::Transport},
};

/// The assignment of the `TES` documentation test.
fn swap_tes(tes: &mut TES, transport: Transport) {
    tes.transport = transport;
}

/// The assignment of the `WES` documentation test.
fn swap_wes(wes: &mut WES, service: ga4gh_sdk::clients::serviceinfo::models::Service) {
    wes.service = service;
}

/// The assignment of the `DRS` documentation test.
fn retarget(drs: &mut DRS, config: Configuration) {
    drs.config = config;
}

/// The assignment of the `Discovered` documentation test.
fn swap_client(discovered: &mut Discovered<TES>, client: TES) {
    discovered.client = client;
}

/// The assignment of the `Transport` documentation test.
fn retime(transport: &mut Transport) {
    transport.config.timeouts.request = std::time::Duration::from_secs(1);
}

#[test]
fn deprecated_fields_are_still_assignable() {
    let config = Configuration::new(url::Url::parse("https://example.invalid/").unwrap());
    let mut transport = Transport::new(&config);
    retime(&mut transport);
    assert_eq!(
        transport.configuration().timeouts.request,
        std::time::Duration::from_secs(1)
    );
    let mut tes = TES::without_discovery(transport.clone(), Default::default());
    swap_tes(&mut tes, Transport::new(&config));
    let mut wes = WES::without_discovery(transport.clone(), Default::default());
    swap_wes(&mut wes, Default::default());
    let mut drs = DRS::without_discovery(transport, Default::default());
    retarget(&mut drs, config);
    // `Discovered` has no public constructor without a request; the function
    // above already proves that the assignment compiles.
    let _ = swap_client;
}
