use ga4gh_sdk::{
    clients::wes::{WorkflowAttachment, WES},
    utils::{
        acknowledgement::AcknowledgementError, configuration::Configuration,
        extension_manager::ExtensionManager, transport::Transport,
    },
};
use ga4gh_workflow::{Error, Identity, Pins, Policy, Workflow};
use serde_json::json;
use std::{env, fs, path::Path};

#[tokio::test]
async fn typed_provider_and_wes_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let mut manager = ExtensionManager::open(dir.path().join("registry.json")).unwrap();
    manager
        .add_extension(env::var("GA4GH_WORKFLOW_MANIFEST").expect("run make workflow-bindings"))
        .await
        .unwrap();
    manager
        .enable_extension("ga4gh-workflow-extension")
        .unwrap();
    let client = unsafe { Workflow::open(&manager).unwrap() };
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../examples/sequence-qc");
    let read = |name| fs::read(example.join(name)).unwrap();
    let profile = read("profile.json");
    let request = serde_json::from_slice(&read("request.json")).unwrap();
    let invalid = serde_json::from_slice(&read("invalid-request.json")).unwrap();
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let pins = Pins {
        profile_revision: pin.into(),
        vocabulary_revision: pin.into(),
    };
    let identity = Identity {
        workflow: "public-qc@1".into(),
        descriptor_digest: None,
    };
    let prepare = || {
        client
            .prepare(&profile, &pins, &request, &identity)
            .unwrap()
    };
    let plan = prepare();
    let record = plan.record().clone();
    plan.release().unwrap();
    let Err(Error::Workflow(failure)) = client.prepare(&profile, &pins, &invalid, &identity) else {
        panic!("expected validation error")
    };
    assert_eq!(failure.code, "validation_failed");
    let outputs = json!({"metrics":{"sequence_count":3,"total_length":12,"mean_length":4},"report":{"class":"File","location":"drs://example/report"}});
    let projected = client
        .results(&profile, &pins, "run-1", "COMPLETE", &outputs)
        .unwrap();
    let verified = client
        .verify(&projected, "/report/location", b"report", None)
        .unwrap();
    assert_eq!(
        verified.artifacts["/report/location"].access_status,
        "available"
    );
    let policy: Policy = serde_json::from_slice(&read("policy.json")).unwrap();
    let report = client.compare(&policy, &verified, &verified).unwrap();
    let snapshot =
        json!({"plan":record,"diagnostics":failure.diagnostics,"record":verified,"report":report});
    if let Some(path) = env::var_os("GA4GH_WORKFLOW_PARITY") {
        fs::write(path, serde_json::to_vec(&snapshot).unwrap()).unwrap()
    }

    let mut server = mockito::Server::new_async().await;
    let config = Configuration::new(server.url().parse().unwrap());
    let wes = WES::without_discovery(Transport::new(&config), Default::default());
    let no_post = server.mock("POST", "/runs").expect(0).create();
    let pinned = Identity {
        workflow: identity.workflow.clone(),
        descriptor_digest: Some(format!("sha256:{}", "0".repeat(64))),
    };
    let wrong = client
        .prepare(&profile, &pins, &request, &pinned)
        .unwrap()
        .submit(
            &wes,
            vec![WorkflowAttachment {
                filename: "main.cwl".into(),
                content: b"wrong".to_vec(),
            }],
        )
        .await;
    assert!(matches!(wrong, Err(Error::Workflow(_))));
    no_post.assert();
    no_post.remove();
    let accepted = server
        .mock("POST", "/runs")
        .match_body(mockito::Matcher::Regex("validated-content".into()))
        .with_body(r#"{"run_id":"accepted-1"}"#)
        .expect(1)
        .create();
    let receipt = prepare()
        .submit(
            &wes,
            vec![WorkflowAttachment {
                filename: "main.cwl".into(),
                content: b"validated-content".to_vec(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(receipt.run_id, "accepted-1");
    assert_eq!(
        receipt.invocation.fingerprint,
        snapshot["plan"]["fingerprint"]
    );
    accepted.assert();
    accepted.remove();
    let missing = server
        .mock("POST", "/runs")
        .with_body("{}")
        .expect(1)
        .create();
    assert!(
        matches!(prepare().submit(&wes,vec![]).await, Err(Error::Wes(cause)) if cause.downcast_ref::<AcknowledgementError>().is_some())
    );
    missing.assert();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Configuration::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let delayed = WES::without_discovery(Transport::new(&config), Default::default());
    {
        let post = prepare().submit(&delayed, vec![]);
        tokio::pin!(post);
        let received = async {
            use tokio::io::AsyncReadExt;
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut prefix = [0; 4];
            socket.read_exact(&mut prefix).await.unwrap();
            assert_eq!(&prefix, b"POST");
            socket
        };
        tokio::select! { biased;
            socket = tokio::time::timeout(std::time::Duration::from_secs(10),received) => {let _socket=socket.unwrap();},
            _ = &mut post => panic!("submission unexpectedly completed"),
        }
    }
    assert!(
        matches!(client.prepare(&profile,&pins,&request,&identity),Err(Error::Workflow(failure)) if failure.code=="state_in_use")
    );
}
