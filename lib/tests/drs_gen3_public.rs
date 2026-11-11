//! Read-only smoke test of the Gen3 bulk dialect against CanineDC.
//!
//! Ignored by default because it needs network access. It sends one anonymous
//! service-info request and one bulk metadata request; it requests no access
//! URLs and downloads no object bytes.

use ga4gh_sdk::clients::drs::{
    bulk::{BulkDialect, BulkKey, BulkPolicy},
    DRS,
};
use ga4gh_sdk::utils::{configuration::Configuration, operation::Operation};
use std::time::Duration;
use url::Url;

/// Public CanineDC object from the Biomedical Research Hub access tutorial.
const FOUND: &str = "dg.C78ne/b2d9ad49-979d-4de2-9f7f-b3d054593de0";
const MISSING: &str = "dg.C78ne/00000000-0000-0000-0000-000000000000";

#[tokio::test]
#[ignore = "needs network access to caninedc.org"]
async fn caninedc_bulk_objects_use_the_gen3_dialect() {
    let base = Url::parse("https://caninedc.org/ga4gh/drs/v1").unwrap();
    let drs = DRS::new(&Configuration::new(base)).await.unwrap();
    let policy = BulkPolicy {
        max_requests: 1,
        ..Default::default()
    };
    let report = drs
        .get_bulk_objects_raw(
            &[FOUND.into(), MISSING.into()],
            &[],
            &policy,
            &Operation::new(Duration::from_secs(30)),
        )
        .await
        .unwrap();

    assert_eq!(
        report.dialects.into_iter().collect::<Vec<_>>(),
        [BulkDialect::Gen3]
    );
    let found = &report.outcomes[&BulkKey {
        object_id: FOUND.into(),
        access_id: None,
    }];
    assert!(found.is_success(), "{found:?}");
    assert_eq!(found.resolved[0]["id"], FOUND);
    let missing = &report.outcomes[&BulkKey {
        object_id: MISSING.into(),
        access_id: None,
    }];
    assert_eq!(missing.failure_codes, vec![404], "{missing:?}");
}
