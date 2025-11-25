const API_BASE = 'http://localhost:3000';

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

// Resource Management
export type ResourceType = 'git' | 'local' | 'web';

export interface Resource {
    id: string;
    name: string;
    resource_type: ResourceType;
    path: string;
    enabled: boolean;
    latest_job?: Job;
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

// Intent Classification
export interface IntentClassifyRequest {
    query: string;
}

export type IntentType =
    | 'fix_bug'
    | 'explain_code'
    | 'refactor_code'
    | 'write_test'
    | 'debug_error'
    | 'generate_doc'
    | 'analyze_performance'
    | 'ask_question'
    | { other: string };

export interface IntentClassifyResponse {
    intent: IntentType;
    confidence: number;
    entities: string[];
    needs_context: boolean;
}

export async function classifyIntent(query: string): Promise<IntentClassifyResponse> {
    const response = await fetch(`${API_BASE}/api/classify`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ query }),
    });

    if (!response.ok) {
        const text = await response.text();
        throw new Error(`Failed to classify intent: ${text}`);
    }

    return response.json();
}

// Prompt Enhancement
export type PromptStrategy = 'full_code' | 'reference_only' | 'architectural';

export interface EnhancePromptRequest {
    query: string;
    strategy?: PromptStrategy;
}

export interface EnhancedPromptResponse {
    original_query: string;
    enhanced_prompt: string;
    intent: IntentType;
    context_chunks: string[];
    preferences_applied: boolean;
}

export async function enhancePrompt(query: string, strategy?: PromptStrategy): Promise<EnhancedPromptResponse> {
    const response = await fetch(`${API_BASE}/api/enhance`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ query, strategy }),
    });

    if (!response.ok) {
        const text = await response.text();
        throw new Error(`Failed to enhance prompt: ${text}`);
    }

    return response.json();
}

// User Preferences
export interface UserPreferences {
    explanation_style?: string;
    code_style?: string;
    documentation_style?: string;
    test_style?: string;
    language_preference?: string;
    verbosity?: string;
}

export interface GetPreferencesResponse {
    preferences: UserPreferences;
}

export async function getPreferences(): Promise<GetPreferencesResponse> {
    const response = await fetch(`${API_BASE}/api/preferences`);

    if (!response.ok) {
        throw new Error(`Failed to get preferences: ${response.statusText}`);
    }

    return response.json();
}

export interface UpdatePreferencesRequest {
    preferences: UserPreferences;
}

export interface UpdatePreferencesResponse {
    success: boolean;
}

export async function updatePreferences(preferences: UserPreferences): Promise<UpdatePreferencesResponse> {
    const response = await fetch(`${API_BASE}/api/preferences`, {
        method: 'PUT',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ preferences }),
    });

    if (!response.ok) {
        throw new Error(`Failed to update preferences: ${response.statusText}`);
    }

    return response.json();
}

// App Status
export interface AppStatusResponse {
    status: 'initializing' | 'ready' | 'error';
    message?: string;
    progress?: string;
}

export async function getAppStatus(): Promise<AppStatusResponse> {
    const response = await fetch(`${API_BASE}/api/status`);

    if (!response.ok) {
        throw new Error(`Failed to get app status: ${response.statusText}`);
    }

    return response.json();
}

export interface RetryInitResponse {
    success: boolean;
    message: string;
}

export async function retryInit(): Promise<RetryInitResponse> {
    const response = await fetch(`${API_BASE}/api/retry_init`, {
        method: 'POST',
    });

    if (!response.ok) {
        throw new Error(`Failed to retry initialization: ${response.statusText}`);
    }

    return response.json();
}

// Source Profile
export interface SourceProfile {
    profile_name: string;
    description: string;
    tech_stack: string[];
    architecture_notes: string[];
    key_conventions: string[];
}

export async function getProfile(sourceId: string): Promise<SourceProfile> {
    const response = await fetch(`${API_BASE}/api/sources/${sourceId}/profile`);
    if (!response.ok) {
        throw new Error(`Failed to get profile: ${response.statusText}`);
    }
    return response.json();
}

export async function updateProfile(sourceId: string, profile: SourceProfile): Promise<void> {
    const response = await fetch(`${API_BASE}/api/sources/${sourceId}/profile`, {
        method: 'PUT',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify(profile),
    });
    if (!response.ok) {
        throw new Error(`Failed to update profile: ${response.statusText}`);
    }
}

export interface GenerateProfileRequest {
    files?: string[];
}

export async function generateProfile(sourceId: string, req: GenerateProfileRequest = {}): Promise<SourceProfile> {
    const response = await fetch(`${API_BASE}/api/sources/${sourceId}/profile/generate`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify(req),
    });
    if (!response.ok) {
        throw new Error(`Failed to generate profile: ${response.statusText}`);
    }
    return response.json();
}
