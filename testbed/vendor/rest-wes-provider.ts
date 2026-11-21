import {
  WesProvider,
  ServiceInfo,
  RunListResponse,
  RunRequest,
  RunId,
  RunLog,
  RunStatus,
  ErrorResponse,
} from "./wes-provider.js";

/**
 * REST API implementation of WesProvider
 * Handles HTTP requests to GA4GH WES v1.0.0 compliant endpoints
 */
export class RestWesProvider implements WesProvider {
  protected baseUrl: string;

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl.endsWith("/") ? baseUrl.slice(0, -1) : baseUrl;
  }

  /**
   * Sends every request of this provider. Override it in a subclass to add
   * headers (e.g. `Authorization`) or otherwise change requests.
   */
  // eslint-disable-next-line class-methods-use-this
  protected fetch(input: RequestInfo | URL, init?: RequestInit) {
    return fetch(input, init);
  }

  private static async handleResponse<T>(response: Response): Promise<T> {
    if (!response.ok) {
      let errorMessage = `HTTP ${response.status}: ${response.statusText}`;

      try {
        const errorBody: ErrorResponse = await response.json();
        if (errorBody.msg) {
          errorMessage = errorBody.msg;
        }
      } catch {
        // If we can't parse the error body, use the default message
      }

      throw new Error(errorMessage);
    }

    return response.json();
  }

  async getServiceInfo(): Promise<ServiceInfo> {
    const response = await this.fetch(`${this.baseUrl}/service-info`);
    return RestWesProvider.handleResponse<ServiceInfo>(response);
  }

  async listRuns(
    pageSize?: number,
    pageToken?: string
  ): Promise<RunListResponse> {
    const params = new URLSearchParams();

    if (pageSize !== undefined) {
      params.append("page_size", pageSize.toString());
    }

    if (pageToken) {
      params.append("page_token", pageToken);
    }

    const url = `${this.baseUrl}/runs${
      params.toString() ? `?${params.toString()}` : ""
    }`;
    const response = await this.fetch(url);
    return RestWesProvider.handleResponse<RunListResponse>(response);
  }

  async runWorkflow(request: RunRequest): Promise<RunId> {
    const formData = new FormData();

    // Add required fields
    formData.append("workflow_params", JSON.stringify(request.workflow_params));
    formData.append("workflow_type", request.workflow_type);
    formData.append("workflow_type_version", request.workflow_type_version);
    formData.append("workflow_url", request.workflow_url);

    // Add optional fields
    if (request.tags) {
      formData.append("tags", JSON.stringify(request.tags));
    }

    if (request.workflow_engine_parameters) {
      formData.append(
        "workflow_engine_parameters",
        JSON.stringify(request.workflow_engine_parameters)
      );
    }

    // Add file attachments
    if (request.workflow_attachment) {
      request.workflow_attachment.forEach((file) => {
        formData.append("workflow_attachment", file);
      });
    }

    const response = await this.fetch(`${this.baseUrl}/runs`, {
      method: "POST",
      body: formData,
    });

    return RestWesProvider.handleResponse<RunId>(response);
  }

  async getRunLog(runId: string): Promise<RunLog> {
    const response = await this.fetch(
      `${this.baseUrl}/runs/${encodeURIComponent(runId)}`
    );
    return RestWesProvider.handleResponse<RunLog>(response);
  }

  async getRunStatus(runId: string): Promise<RunStatus> {
    const response = await this.fetch(
      `${this.baseUrl}/runs/${encodeURIComponent(runId)}/status`
    );
    return RestWesProvider.handleResponse<RunStatus>(response);
  }

  async cancelRun(runId: string): Promise<RunId> {
    const response = await this.fetch(
      `${this.baseUrl}/runs/${encodeURIComponent(runId)}/cancel`,
      {
        method: "POST",
      }
    );
    return RestWesProvider.handleResponse<RunId>(response);
  }
}
