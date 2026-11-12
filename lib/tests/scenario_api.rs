//! Opt-in cross-client scenarios against the shared, nonexecuting API.
use ga4gh_sdk::{
    clients::{
        drs::uri::{AccessPreferences, AccessResolution, DrsResolver},
        tes::{models::TesTask, TES},
        trs::{models::DescriptorTypeWithPlain, TRS},
        wes::{models::RunRequest, WorkflowAttachment, WES},
    },
    utils::{
        configuration::Configuration,
        operation::{AmbiguousSubmissionError, Operation},
        transport::Transport,
    },
};
use serde_json::{json, Value};

struct Session {
    base: String,
    id: String,
    token: String,
    client: reqwest::Client,
}
impl Session {
    async fn new(base: &str) -> Self {
        let client = reqwest::Client::new();
        let s: Value = client
            .post(format!("{base}/control/v1/sessions"))
            .json(&json!({"seed":17}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        Self {
            base: base.into(),
            id: s["session_id"].as_str().unwrap().into(),
            token: s["token"].as_str().unwrap().into(),
            client,
        }
    }
    fn config(&self, service: &str) -> Configuration {
        let mut config = Configuration::new(
            url::Url::parse(&format!(
                "{}/sessions/{}/{}/{}",
                self.base,
                self.id,
                service,
                if service == "trs" { "v2" } else { "v1" }
            ))
            .unwrap(),
        );
        config.bearer_access_token = Some(self.token.clone());
        config
    }
    async fn transport(&self, service: &str) -> Transport {
        Transport::new(&self.config(service))
    }
    async fn wes(&self) -> WES {
        let transport = self.transport("wes").await;
        let service =
            serde_json::from_str(&transport.get("service-info", None).await.unwrap()).unwrap();
        WES {
            config: self.config("wes"),
            service: Ok(service),
            transport,
        }
    }
    async fn tes(&self) -> TES {
        let transport = self.transport("tes").await;
        let service =
            serde_json::from_str(&transport.get("service-info", None).await.unwrap()).unwrap();
        TES {
            config: self.config("tes"),
            service: Ok(service),
            transport,
        }
    }
    async fn control(&self, action: Value) {
        self.client
            .post(format!(
                "{}/control/v1/sessions/{}/events",
                self.base, self.id
            ))
            .bearer_auth(&self.token)
            .json(&action)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    async fn journal(&self) -> Value {
        self.client
            .get(format!(
                "{}/control/v1/sessions/{}/replay",
                self.base, self.id
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}
fn request() -> RunRequest {
    RunRequest {
        workflow_params: Some(json!({"message":"synthetic"})),
        workflow_type: "CWL".into(),
        workflow_type_version: "v1.2".into(),
        workflow_url: "fixture.cwl".into(),
        ..Default::default()
    }
}
fn attachment() -> Vec<WorkflowAttachment> {
    vec![WorkflowAttachment {
        filename: "fixture.cwl".into(),
        content: include_bytes!("../../testbed/pack/fixture.cwl").to_vec(),
    }]
}
#[tokio::test]
#[ignore = "requires tests/run-h3-scenarios.py"]
async fn sdk_shared_scenarios() {
    let base = std::env::var("GA4GH_SCENARIO_API").unwrap();
    let s = Session::new(&base).await;
    let wes = s.wes().await;
    let mut unsupported = request();
    unsupported.workflow_type_version = "unsupported".into();
    assert!(wes
        .run_workflow_with_attachments(unsupported, attachment())
        .await
        .is_err());
    let a = wes
        .run_workflow_with_attachments(request(), attachment())
        .await
        .unwrap();
    let b = wes
        .run_workflow_with_attachments(request(), attachment())
        .await
        .unwrap();
    let page = wes.list_runs(None, Some(1)).await.unwrap();
    assert_eq!(page.runs.unwrap()[0].run_id.as_str(), a.id.as_str());
    let page2 = wes.list_runs(page.next_page_token, Some(1)).await.unwrap();
    assert_eq!(page2.runs.unwrap()[0].run_id.as_str(), b.id.as_str());
    s.control(json!({"action":"advance"})).await;
    assert_eq!(a.status().await.unwrap().to_string(), "RUNNING");
    s.control(json!({"action":"fault","operation":"wes.cancel","name":"cancel-completion-race"}))
        .await;
    a.cancel().await.unwrap();
    assert_eq!(a.status().await.unwrap().to_string(), "COMPLETE");
    b.cancel().await.unwrap();
    assert_eq!(b.status().await.unwrap().to_string(), "CANCELING");
    s.control(json!({"action":"advance"})).await;
    assert_eq!(b.status().await.unwrap().to_string(), "CANCELED");
    assert_eq!(
        a.log().await.unwrap().run_id.as_deref(),
        Some(a.id.as_str())
    );
    s.control(json!({"action":"fault","operation":"wes.create","name":"lost-response"}))
        .await;
    let unknown = wes
        .run_workflow_with_attachments(request(), attachment())
        .await
        .unwrap_err();
    assert!(unknown.downcast_ref::<AmbiguousSubmissionError>().is_some());
    assert_eq!(s.journal().await["accepted_runs"], 3);
    s.control(json!({"action":"fault","operation":"wes.list","name":"503"}))
        .await;
    assert!(wes.list_runs(None, None).await.is_err());
    assert!(wes.list_runs(None, None).await.is_ok());
    s.control(json!({"action":"fault","operation":"wes.list","name":"malformed-json"}))
        .await;
    assert!(wes.list_runs(None, None).await.is_err());
    let tes = s.tes().await;
    let task = || {
        serde_json::from_value::<TesTask>(
            json!({"executors":[{"image":"synthetic","command":["fixture"]}]}),
        )
        .unwrap()
    };
    let t = tes.create(task()).await.unwrap();
    assert_eq!(t.status().await.unwrap().to_string(), "QUEUED");
    t.cancel().await.unwrap();
    s.control(json!({"action":"advance"})).await;
    assert_eq!(t.status().await.unwrap().to_string(), "CANCELED");
    s.control(json!({"action":"fault","operation":"tes.create","name":"lost-response"}))
        .await;
    assert!(tes
        .create(task())
        .await
        .unwrap_err()
        .downcast_ref::<AmbiguousSubmissionError>()
        .is_some());
    let data_session = Session::new(&base).await;
    let resolver = DrsResolver::default()
        .with_service("synthetic", data_session.transport("drs").await)
        .unwrap();
    let budget = || Operation::new(std::time::Duration::from_secs(5));
    assert!(matches!(
        resolver
            .access_plan_once(
                "drs://synthetic/fixture",
                &AccessPreferences::default(),
                &budget()
            )
            .await
            .unwrap(),
        AccessResolution::Pending(_)
    ));
    data_session.control(json!({"action":"advance"})).await;
    let plan = match resolver
        .access_plan_once(
            "drs://synthetic/fixture",
            &AccessPreferences::default(),
            &budget(),
        )
        .await
        .unwrap()
    {
        AccessResolution::Ready(plan) => plan,
        _ => panic!("fixture still pending"),
    };
    let bytes = reqwest::Client::new()
        .get(plan.url.clone())
        .headers(plan.headers.clone())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let expected = (0u8..=255)
        .chain(b"\0GA4GH\xff\n".iter().copied())
        .collect::<Vec<_>>();
    assert_eq!(bytes.as_ref(), expected.as_slice());
    data_session
        .control(json!({"action":"advance","ticks":9}))
        .await;
    assert_eq!(
        reqwest::Client::new()
            .get(plan.url)
            .headers(plan.headers)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    let trs_transport = data_session.transport("trs").await;
    let trs = TRS::with_transport(trs_transport).await.unwrap();
    let wrapper = trs
        .get_descriptor("fixture", "1", DescriptorTypeWithPlain::Cwl)
        .await
        .unwrap();
    assert_eq!(
        wrapper.content.unwrap().as_bytes(),
        include_bytes!("../../testbed/pack/fixture.cwl")
    );

    let replay = s.journal().await;
    assert_eq!(replay["accepted_tasks"], 2);
    assert_eq!(replay["accepted_runs"], 3);
    if let Ok(path) = std::env::var("GA4GH_SCENARIO_SDK_EVIDENCE") {
        std::fs::write(path,serde_json::to_vec_pretty(&json!({"client":"ga4gh-sdk","evidence_class":"simulated-protocol","scenarios":["multipart","opaque-paging","cancel-completion-race","cancel-pending","lost-response","bounded-overload","malformed-json","tes-lifecycle"],"replay":replay})).unwrap()).unwrap();
    }
}
