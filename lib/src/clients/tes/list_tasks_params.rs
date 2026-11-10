use crate::clients::tes::models;

/// struct for passing parameters to the method [`list_tasks`]
#[derive(Serialize, Clone, Debug)]
pub struct ListTasksParams {
    /// OPTIONAL. Filter the list to include tasks where the name matches this prefix. If unspecified, no task name filtering is done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_prefix: Option<String>,
    /// OPTIONAL. Filter tasks by state. If unspecified, no task state filtering is done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<models::TesState>,
    /// OPTIONAL. Provide key tag to filter. The field tag_key is an array of key values, and will be zipped with an optional tag_value array. So the query: ```   ?tag_key=foo1&tag_value=bar1&tag_key=foo2&tag_value=bar2 ``` Should be constructed into the structure { \"foo1\" : \"bar1\", \"foo2\" : \"bar2\"}  ```   ?tag_key=foo1 ``` Should be constructed into the structure {\"foo1\" : \"\"}  If the tag_value is empty, it will be treated as matching any possible value. If a tag value is provided, both the tag's key and value must be exact matches for a task to be returned. Filter                            Tags                          Match? ---------------------------------------------------------------------- {\"foo\": \"bar\"}                    {\"foo\": \"bar\"}                Yes {\"foo\": \"bar\"}                    {\"foo\": \"bat\"}                No {\"foo\": \"\"}                       {\"foo\": \"\"}                   Yes {\"foo\": \"bar\", \"baz\": \"bat\"}      {\"foo\": \"bar\", \"baz\": \"bat\"}  Yes {\"foo\": \"bar\"}                    {\"foo\": \"bar\", \"baz\": \"bat\"}  Yes {\"foo\": \"bar\", \"baz\": \"bat\"}      {\"foo\": \"bar\"}                No {\"foo\": \"\"}                       {\"foo\": \"bar\"}                Yes {\"foo\": \"\"}                       {}                            No
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_key: Option<Vec<String>>,
    /// OPTIONAL. The companion value field for tag_key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_value: Option<Vec<String>>,
    /// Optional number of tasks to return in one page. Must be less than 2048. Defaults to 256.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_size: Option<i32>,
    /// OPTIONAL. Page token is used to retrieve the next page of results. If unspecified, returns the first page of results. The value can be found in the `next_page_token` field of the last returned result of ListTasks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    /// OPTIONAL. Affects the fields included in the returned Task messages.  `MINIMAL`: Task message will include ONLY the fields: - `tesTask.Id` - `tesTask.State`  `BASIC`: Task message will include all fields EXCEPT: - `tesTask.ExecutorLog.stdout` - `tesTask.ExecutorLog.stderr` - `tesInput.content` - `tesTaskLog.system_logs`  `FULL`: Task message includes all fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
}
