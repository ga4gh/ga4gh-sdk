#[cfg(feature = "integration_tests")]
#[cfg(test)]
mod tests {
    use ga4gh_sdk::clients::tes::models::ListTasksParams;
    use ga4gh_sdk::clients::tes::models::TesState;
    use ga4gh_sdk::clients::tes::models::TesTask;
    use ga4gh_sdk::clients::tes::Task;
    use ga4gh_sdk::clients::tes::TES;
    use ga4gh_sdk::utils::configuration::Configuration;
    use ga4gh_sdk::utils::test_utils::{ensure_funnel_running, setup};

    async fn create_task() -> Result<(Task, TES), Box<dyn std::error::Error>> {
        // setup(); – should be run once in the test function
        let mut config = Configuration::default();
        let funnel_url = ensure_funnel_running().await;
        let funnel_url = url::Url::parse(&funnel_url).expect("Invalid URL");
        config.set_base_path(funnel_url);
        let tes = match TES::new(&config).await {
            Ok(tes) => tes,
            Err(e) => {
                println!("Error creating TES instance: {:?}", e);
                return Err(e);
            }
        };
        let file_path = "../tests/sample.tes".to_string();
        let task_json = match std::fs::read_to_string(&file_path) {
            Ok(content) => content,
            Err(e) => {
                let current_dir = std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("unknown directory"));
                panic!(
                    "Unable to read file in directory {:?}: {:?}",
                    current_dir, e
                );
            }
        };
        let task: TesTask = serde_json::from_str(&task_json).expect("JSON was not well-formatted");

        let task = tes.create(task).await?;
        Ok((task, tes))
    }

    #[tokio::test]
    async fn test_task_create() {
        setup();
        let (task, _tes) = create_task().await.expect("Failed to create task");
        assert!(!task.id.is_empty(), "Task ID should not be empty"); // double check if it's a correct assertion
    }

    #[tokio::test]
    async fn test_task_status() {
        setup();

        let (task, _tes) = create_task().await.expect("Failed to create task");
        assert!(!task.id.is_empty(), "Task ID should not be empty");

        let status = task.status().await;
        match status {
            Ok(state) => {
                assert!(
                    matches!(
                        state,
                        TesState::Initializing | TesState::Queued | TesState::Running
                    ),
                    "Unexpected state: {:?}",
                    state
                );
            }
            Err(err) => {
                panic!("Task status returned an error: {:?}", err);
            }
        }
    }

    #[tokio::test]
    async fn test_cancel_task() {
        setup();

        let (task, _tes) = &create_task().await.expect("Failed to create task");
        assert!(!task.id.is_empty(), "Task ID should not be empty"); // double check if it's a correct assertion

        let cancel = task.cancel().await;
        assert!(cancel.is_ok());
    }

    #[tokio::test]
    async fn test_list_task() {
        setup();

        let (task, tes) = &create_task().await.expect("Failed to create task");
        assert!(!task.id.is_empty(), "Task ID should not be empty"); // double check if it's a correct assertion

        let params: ListTasksParams = ListTasksParams {
            name_prefix: None,
            state: None,
            tag_key: None,
            tag_value: None,
            page_size: None,
            page_token: None,
            view: Some("BASIC".to_string()),
        };

        let list = tes.list_tasks(Some(params)).await;
        assert!(list.is_ok());
        println!("{:?}", list);
    }
}
