/**
 * Types for the GA4GH WES API based on OpenAPI specification v1.0.0
 */

// Workflow execution states
export type State =
  | "UNKNOWN"
  | "QUEUED"
  | "INITIALIZING"
  | "RUNNING"
  | "PAUSED"
  | "COMPLETE"
  | "EXECUTOR_ERROR"
  | "SYSTEM_ERROR"
  | "CANCELED"
  | "CANCELING";

// Workflow types
export type WorkflowType = "CWL" | "WDL" | "NFL" | "GALAXY" | "SMK" | string;

// Default workflow engine parameter
export interface DefaultWorkflowEngineParameter {
  name: string;
  type: string;
  default_value: string;
}

// Log information for tasks and workflows
export interface Log {
  name?: string;
  cmd?: string[];
  start_time?: string;
  end_time?: string;
  stdout?: string;
  stderr?: string;
  exit_code?: number;
}

// Workflow type version
export interface WorkflowTypeVersion {
  workflow_type_version: string[];
}

// Service information
export interface ServiceInfo {
  workflow_type_versions: Record<string, WorkflowTypeVersion>;
  supported_wes_versions: string[];
  supported_filesystem_protocols: string[];
  workflow_engine_versions: Record<string, string>;
  default_workflow_engine_parameters: DefaultWorkflowEngineParameter[];
  system_state_counts: Record<string, number>;
  auth_instructions_url?: string;
  contact_info_url?: string;
  tags?: Record<string, string>;
}

// Run request for submitting workflows
export interface RunRequest {
  workflow_params: object;
  workflow_type: string;
  workflow_type_version: string;
  tags?: Record<string, string>;
  workflow_engine_parameters?: Record<string, string>;
  workflow_url: string;
  workflow_attachment?: File[];
}

// Run ID response
export interface RunId {
  run_id: string;
}

// Run status (abbreviated info)
export interface RunStatus {
  run_id: string;
  state: State;
}

// Complete run log with all details
export interface RunLog {
  run_id: string;
  request: RunRequest;
  state: State;
  run_log: Log;
  task_logs: Log[];
  outputs: object;
}

// Run list response with pagination
export interface RunListResponse {
  runs: RunStatus[];
  next_page_token?: string;
}

// Error response
export interface ErrorResponse {
  msg?: string;
  status_code?: number;
}

/**
 * Interface defining the operations required for WES data providers
 * Implementations could use REST API, GraphQL, or any other data source
 */
export interface WesProvider {
  // Service information
  getServiceInfo(): Promise<ServiceInfo>;

  // Run management
  listRuns(pageSize?: number, pageToken?: string): Promise<RunListResponse>;
  runWorkflow(request: RunRequest): Promise<RunId>;
  getRunLog(runId: string): Promise<RunLog>;
  getRunStatus(runId: string): Promise<RunStatus>;
  cancelRun(runId: string): Promise<RunId>;
}
