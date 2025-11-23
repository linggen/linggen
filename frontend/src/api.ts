const API_BASE = 'http://localhost:3000';

export interface IndexRequest {
    document_id: string;
    content: string;
}

export interface IndexResponse {
    chunks_indexed: number;
    document_id: string;
}

export interface SearchResult {
    document_id: string;
    content: string;
    score: number;
}

export interface SearchResponse {
    results: SearchResult[];
    query: string;
}

export async function indexDocument(req: IndexRequest): Promise<IndexResponse> {
    const response = await fetch(`${API_BASE}/api/index`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify(req),
    });

    if (!response.ok) {
        throw new Error(`Failed to index document: ${response.statusText}`);
    }

    return response.json();
}

export async function searchDocuments(query: string, limit: number = 10): Promise<SearchResponse> {
    const response = await fetch(`${API_BASE}/api/search?q=${encodeURIComponent(query)}&limit=${limit}`);

    if (!response.ok) {
        throw new Error(`Search failed: ${response.statusText}`);
    }

    return response.json();
}

// Resource Management
export type ResourceType = 'git' | 'local' | 'web';

export interface Resource {
    id: string;
    name: string;
    resource_type: ResourceType;
    path: string;
    enabled: boolean;
}

export interface AddResourceRequest {
    name: string;
    resource_type: ResourceType;
    path: string;
}

export interface AddResourceResponse {
    id: string;
    name: string;
    resource_type: ResourceType;
    path: string;
    enabled: boolean;
}

export interface ListResourcesResponse {
    resources: Resource[];
}

export interface RemoveResourceResponse {
    success: boolean;
    id: string;
}

export async function addResource(req: AddResourceRequest): Promise<AddResourceResponse> {
    const response = await fetch(`${API_BASE}/api/resources`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify(req),
    });

    if (!response.ok) {
        throw new Error(`Failed to add resource: ${response.statusText}`);
    }

    return response.json();
}

export async function listResources(): Promise<ListResourcesResponse> {
    const response = await fetch(`${API_BASE}/api/resources`);

    if (!response.ok) {
        throw new Error(`Failed to list resources: ${response.statusText}`);
    }

    return response.json();
}

export async function removeResource(id: string): Promise<RemoveResourceResponse> {
    const response = await fetch(`${API_BASE}/api/resources/remove`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ id }),
    });

    if (!response.ok) {
        throw new Error(`Failed to remove resource: ${response.statusText}`);
    }

    return response.json();
}

// Indexing
export interface IndexFolderResponse {
    files_indexed: number;
    chunks_created: number;
    folder_path: string;
}

export async function indexFolder(folderPath: string): Promise<IndexFolderResponse> {
    const response = await fetch(`${API_BASE}/api/index_folder`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ folder_path: folderPath }),
    });

    if (!response.ok) {
        throw new Error(`Failed to index folder: ${response.statusText}`);
    }

    return response.json();
}

// Index Source (with job tracking)
export interface IndexSourceRequest {
    source_id: string;
}

export interface IndexSourceResponse {
    job_id: string;
    files_indexed: number;
    chunks_created: number;
}

export async function indexSource(sourceId: string): Promise<IndexSourceResponse> {
    const response = await fetch(`${API_BASE}/api/index_source`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ source_id: sourceId }),
    });

    if (!response.ok) {
        const text = await response.text();
        throw new Error(`Failed to index source: ${text}`);
    }

    return response.json();
}

// Jobs
export type JobStatus = 'Pending' | 'Running' | 'Completed' | 'Failed';

export interface Job {
    id: string;
    source_id: string;
    source_name: string;
    source_type: ResourceType;
    status: JobStatus;
    started_at: string;
    finished_at?: string;
    files_indexed?: number;
    chunks_created?: number;
    total_files?: number;
    total_size_bytes?: number;
    error?: string;
}

export interface ListJobsResponse {
    jobs: Job[];
}

export async function listJobs(): Promise<ListJobsResponse> {
    const response = await fetch(`${API_BASE}/api/jobs`);

    if (!response.ok) {
        throw new Error(`Failed to list jobs: ${response.statusText}`);
    }

    return response.json();
}

export interface CancelJobRequest {
    job_id: string;
}

export interface CancelJobResponse {
    success: boolean;
    job_id: string;
}

export async function cancelJob(jobId: string): Promise<CancelJobResponse> {
    const response = await fetch(`${API_BASE}/api/jobs/cancel`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ job_id: jobId }),
    });

    if (!response.ok) {
        throw new Error(`Failed to cancel job: ${response.statusText}`);
    }

    return response.json();
}
