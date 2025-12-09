
// Determine the API base URL:
// - In Tauri desktop app (any non-http/https protocol or Tauri UA), always use localhost:8787
// - In browser on localhost:8787, use localhost:8787
// - In browser on remote IP (http://linggen-ip:8787), use that IP
function getApiBase(): string {
    if (typeof window === 'undefined') {
        return 'http://localhost:8787';
    }

    const origin = window.location.origin;
    const protocol = window.location.protocol;

    // Detect Tauri more robustly
    const isTauriProtocol =
        protocol !== 'http:' && protocol !== 'https:'; // e.g. app://, tauri://, file://

    // @ts-expect-error - __TAURI__ is injected by Tauri
    const hasTauriGlobal = typeof window.__TAURI__ !== 'undefined';

    const ua = navigator.userAgent.toLowerCase();
    const isTauriUA = ua.includes('tauri');

    const isTauriEnv =
        isTauriProtocol ||
        hasTauriGlobal ||
        isTauriUA ||
        origin.startsWith('tauri://') ||
        origin.startsWith('app://') ||
        origin === 'null';

    if (isTauriEnv) {
        return 'http://localhost:8787';
    }

    // In Vite dev server (localhost:5173), point to backend
    if (origin.includes('localhost:5173') || origin.includes('127.0.0.1:5173')) {
        return 'http://localhost:8787';
    }

    // For browser access (local or remote), use the current origin
    return origin;
}

const API_BASE = getApiBase();

/** Indexing mode: "full" rebuilds everything, "incremental" only updates changed files */
export type IndexMode = 'full' | 'incremental';

export interface IndexSourceRequest {
    source_id: string;
    mode?: IndexMode;
}

export interface IndexSourceResponse {
    job_id: string;
    files_indexed: number;
    chunks_created: number;
}

/**
 * Index a source with optional mode.
 * @param sourceId The source ID to index
 * @param mode "incremental" (default) only indexes changed files, "full" rebuilds everything
 */
export async function indexSource(sourceId: string, mode: IndexMode = 'incremental'): Promise<IndexSourceResponse> {
    const response = await fetch(`${API_BASE}/api/index_source`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ source_id: sourceId, mode }),
    });

    if (!response.ok) {
        const text = await response.text();
        throw new Error(`Failed to index source: ${text} `);
    }

    return response.json();
}

// Resource Management
export type ResourceType = 'git' | 'local' | 'web' | 'uploads';

export interface SourceStats {
    chunk_count: number;
    file_count: number;
    total_size_bytes: number;
}

export interface Resource {
    id: string;
    name: string;
    resource_type: ResourceType;
    path: string;
    enabled: boolean;
    include_patterns: string[];
    exclude_patterns: string[];
    latest_job?: Job;
    stats?: SourceStats;
    last_upload_time?: string;
}

export interface AddResourceRequest {
    name: string;
    resource_type: ResourceType;
    path: string;
    include_patterns?: string[];
    exclude_patterns?: string[];
}

export interface AddResourceResponse {
    id: string;
    name: string;
    resource_type: ResourceType;
    path: string;
    enabled: boolean;
    include_patterns: string[];
    exclude_patterns: string[];
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
        throw new Error(`Failed to add resource: ${response.statusText} `);
    }

    return response.json();
}

export async function listResources(): Promise<ListResourcesResponse> {
    const response = await fetch(`${API_BASE}/api/resources`);

    if (!response.ok) {
        throw new Error(`Failed to list resources: ${response.statusText} `);
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
        throw new Error(`Failed to remove resource: ${response.statusText} `);
    }

    return response.json();
}

export interface RenameResourceResponse {
    success: boolean;
    id: string;
    name: string;
}

export async function renameResource(id: string, name: string): Promise<RenameResourceResponse> {
    const response = await fetch(`${API_BASE}/api/resources/rename`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ id, name }),
    });

    if (!response.ok) {
        throw new Error(`Failed to rename resource: ${response.statusText} `);
    }

    return response.json();
}

export interface UpdateResourcePatternsResponse {
    success: boolean;
    id: string;
    include_patterns: string[];
    exclude_patterns: string[];
}

export async function updateResourcePatterns(
    id: string,
    include_patterns: string[],
    exclude_patterns: string[]
): Promise<UpdateResourcePatternsResponse> {
    const response = await fetch(`${API_BASE}/api/resources/patterns`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ id, include_patterns, exclude_patterns }),
    });

    if (!response.ok) {
        throw new Error(`Failed to update patterns: ${response.statusText} `);
    }

    return response.json();
}

// File Upload for Uploads sources
export interface UploadFileResponse {
    success: boolean;
    source_id: string;
    filename: string;
    chunks_created: number;
}

export interface UploadProgressInfo {
    phase: string;
    progress: number;
    message: string;
    error?: string;
    result?: UploadFileResponse;
}

// Upload with streaming progress (shows extracting, chunking, embedding progress)
export async function uploadFileWithProgress(
    sourceId: string,
    file: File,
    onProgress?: (info: UploadProgressInfo) => void
): Promise<UploadFileResponse> {
    const formData = new FormData();
    formData.append('source_id', sourceId);
    formData.append('file', file);

    const response = await fetch(`${API_BASE}/api/upload/stream`, {
        method: 'POST',
        body: formData,
    });

    if (!response.ok) {
        throw new Error(`Upload failed: ${response.statusText} `);
    }

    const reader = response.body?.getReader();
    if (!reader) {
        throw new Error('No response body');
    }

    const decoder = new TextDecoder();
    let buffer = '';
    let result: UploadFileResponse | null = null;

    while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });

        // Parse SSE events (data: {...}\n\n)
        const lines = buffer.split('\n\n');
        buffer = lines.pop() || ''; // Keep incomplete data in buffer

        for (const line of lines) {
            if (line.startsWith('data: ')) {
                try {
                    const data = JSON.parse(line.slice(6)) as UploadProgressInfo;
                    if (onProgress) {
                        onProgress(data);
                    }
                    if (data.error) {
                        throw new Error(data.error);
                    }
                    if (data.result) {
                        result = data.result;
                    }
                } catch (e) {
                    if (e instanceof Error && e.message !== 'Unexpected end of JSON input') {
                        throw e;
                    }
                }
            }
        }
    }

    if (!result) {
        throw new Error('Upload completed but no result received');
    }

    return result;
}

// Simple upload without detailed progress (legacy)
export async function uploadFile(
    sourceId: string,
    file: File,
    onProgress?: (percent: number) => void
): Promise<UploadFileResponse> {
    const formData = new FormData();
    formData.append('source_id', sourceId);
    formData.append('file', file);

    return new Promise((resolve, reject) => {
        const xhr = new XMLHttpRequest();

        xhr.upload.addEventListener('progress', (event) => {
            if (event.lengthComputable && onProgress) {
                const percent = Math.round((event.loaded / event.total) * 100);
                onProgress(percent);
            }
        });

        xhr.addEventListener('load', () => {
            if (xhr.status >= 200 && xhr.status < 300) {
                try {
                    const response = JSON.parse(xhr.responseText);
                    resolve(response);
                } catch {
                    reject(new Error('Failed to parse response'));
                }
            } else {
                try {
                    const errorData = JSON.parse(xhr.responseText);
                    reject(new Error(errorData.error || `Upload failed: ${xhr.statusText} `));
                } catch {
                    reject(new Error(`Upload failed: ${xhr.statusText} `));
                }
            }
        });

        xhr.addEventListener('error', () => {
            reject(new Error('Network error during upload'));
        });

        xhr.open('POST', `${API_BASE}/api/upload`);
        xhr.send(formData);
    });
}

// List uploaded files for a source
export interface FileInfo {
    filename: string;
    chunk_count: number;
}

export interface ListFilesResponse {
    source_id: string;
    files: FileInfo[];
}

export async function listUploadedFiles(sourceId: string): Promise<ListFilesResponse> {
    const response = await fetch(`${API_BASE}/api/upload/files`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ source_id: sourceId }),
    });

    if (!response.ok) {
        throw new Error(`Failed to list files: ${response.statusText} `);
    }

    return response.json();
}

// Delete an uploaded file
export interface DeleteFileResponse {
    success: boolean;
    source_id: string;
    filename: string;
    chunks_deleted: number;
}

export async function deleteUploadedFile(sourceId: string, filename: string): Promise<DeleteFileResponse> {
    const response = await fetch(`${API_BASE}/api/upload/delete`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ source_id: sourceId, filename }),
    });

    if (!response.ok) {
        throw new Error(`Failed to delete file: ${response.statusText} `);
    }

    return response.json();
}

// Design Notes API
export interface NoteContent {
    path: string;
    content: string;
    linked_node?: string;
}

export interface Note {
    name: string;
    path: string; // Relative to .linggen/notes
    modified_at?: string;
}

export interface ListNotesResponse {
    notes: Note[];
}

export async function listNotes(sourceId: string): Promise<Note[]> {
    const response = await fetch(`${API_BASE}/api/sources/${sourceId}/notes`);
    if (!response.ok) {
        if (response.status === 404) {
            return [];
        }
        throw new Error(`Failed to list notes: ${response.statusText}`);
    }
    const data: ListNotesResponse = await response.json();
    return data.notes;
}

export async function getNote(sourceId: string, notePath: string): Promise<NoteContent> {
    const response = await fetch(`${API_BASE}/api/sources/${sourceId}/notes/${encodeURIComponent(notePath)}`);
    if (!response.ok) {
        throw new Error(`Failed to get note: ${response.statusText}`);
    }
    return response.json();
}

export async function deleteNote(sourceId: string, notePath: string): Promise<void> {
    const response = await fetch(`${API_BASE}/api/sources/${sourceId}/notes/${encodeURIComponent(notePath)}`, {
        method: 'DELETE',
    });

    if (!response.ok) {
        throw new Error(`Failed to delete note: ${response.statusText}`);
    }
}

export interface SaveNoteRequest {
    content: string;
    linked_node?: string;
}

export async function saveNote(sourceId: string, notePath: string, content: string): Promise<void> {
    const response = await fetch(`${API_BASE}/api/sources/${sourceId}/notes/${notePath}`, {
        method: 'PUT',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ content }),
    });

    if (!response.ok) {
        throw new Error(`Failed to save note: ${response.statusText}`);
    }
}

export interface GraphStatusResponse {
    status: 'missing' | 'stale' | 'ready' | 'building' | 'error';
    node_count?: number;
    edge_count?: number;
    built_at?: string;
}



// Delete an uploaded file


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
    context_metadata?: {
        source_id: string;
        document_id: string;
        file_path: string;
    }[];
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

export interface ChatResponse {
    response: string;
}

/**
 * Stream chat response from the backend
 * 
 * @param message User message
 * @param onToken Callback for each new token
 * @param context Optional context
 */
export async function chatStream(
    message: string,
    onToken: (token: string) => void,
    context?: string
): Promise<void> {
    const response = await fetch(`${API_BASE}/api/chat/stream`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ message, context }),
    });

    if (!response.ok) {
        const text = await response.text();
        throw new Error(`Failed to stream chat: ${text}`);
    }

    if (!response.body) {
        throw new Error('Response body is null');
    }

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        // Append new chunk to buffer
        buffer += decoder.decode(value, { stream: true });

        // Process complete lines from buffer
        const lines = buffer.split('\n');
        // Keep the last (potentially incomplete) line in buffer
        buffer = lines.pop() || '';

        for (const line of lines) {
            if (line.startsWith('data:')) {
                // Per SSE spec there may be an optional single space after "data:"
                // We want to remove at most one leading space, but keep any spaces
                // that are part of the actual payload (important for tokens that
                // encode leading spaces, e.g. " How").
                let data = line.slice(5);
                if (data.startsWith(' ')) {
                    data = data.slice(1);
                }
                if (data.length > 0) {
                    onToken(data);
                }
            }
        }
    }

    // Process any remaining data in buffer
    if (buffer.startsWith('data:')) {
        let data = buffer.slice(5);
        if (data.startsWith(' ')) {
            data = data.slice(1);
        }
        if (data.length > 0) {
            onToken(data);
        }
    }
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

// App Settings
export interface AppSettings {
    intent_detection_enabled: boolean;
    llm_enabled: boolean;
    server_port?: number;
    server_address?: string;
    /** Whether anonymous analytics is enabled (default: true) */
    analytics_enabled: boolean;
}

export async function getAppSettings(): Promise<AppSettings> {
    const response = await fetch(`${API_BASE}/api/settings`);
    if (!response.ok) {
        throw new Error(`Failed to get settings: ${response.statusText}`);
    }
    return response.json();
}

export async function updateAppSettings(settings: AppSettings): Promise<void> {
    const response = await fetch(`${API_BASE}/api/settings`, {
        method: 'PUT',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify(settings),
    });
    if (!response.ok) {
        throw new Error(`Failed to update settings: ${response.statusText}`);
    }
}

export async function clearAllData(): Promise<void> {
    const response = await fetch(`${API_BASE}/api/clear_all_data`, {
        method: 'POST',
    });
    if (!response.ok) {
        const text = await response.text();
        throw new Error(`Failed to clear data: ${text}`);
    }
}



// Graph (Architect) API
export interface GraphNode {
    id: string;
    label: string;
    language: string;
    folder: string;
}

export interface GraphEdge {
    source: string;
    target: string;
    kind: string;
}

export interface GraphResponse {
    project_id: string;
    nodes: GraphNode[];
    edges: GraphEdge[];
    built_at?: string;
}

export interface GraphStatusResponse {
    status: 'missing' | 'stale' | 'ready' | 'building' | 'error';
    node_count?: number;
    edge_count?: number;
    built_at?: string;
}



export interface GraphQuery {
    folder?: string;
    focus?: string;
    hops?: number;
}

export async function getGraphStatus(sourceId: string): Promise<GraphStatusResponse> {
    const response = await fetch(`${API_BASE}/api/sources/${sourceId}/graph/status`);
    if (!response.ok) {
        throw new Error(`Failed to get graph status: ${response.statusText}`);
    }
    return response.json();
}

export async function getGraph(sourceId: string, query?: GraphQuery): Promise<GraphResponse> {
    const params = new URLSearchParams();
    if (query?.folder) params.set('folder', query.folder);
    if (query?.focus) params.set('focus', query.focus);
    if (query?.hops) params.set('hops', query.hops.toString());

    const url = `${API_BASE}/api/sources/${sourceId}/graph${params.toString() ? '?' + params.toString() : ''}`;
    const response = await fetch(url);
    if (!response.ok) {
        throw new Error(`Failed to get graph: ${response.statusText}`);
    }
    return response.json();
}

export async function rebuildGraph(sourceId: string): Promise<GraphStatusResponse> {
    const response = await fetch(`${API_BASE}/api/sources/${sourceId}/graph/rebuild`, {
        method: 'POST',
    });
    if (!response.ok) {
        throw new Error(`Failed to rebuild graph: ${response.statusText}`);
    }
    return response.json();
}

// Combined graph + status response (optimized single request)
export interface GraphWithStatusResponse {
    status: string;
    node_count: number;
    edge_count: number;
    built_at: string | null;
    project_id: string;
    nodes: GraphNode[];
    edges: GraphEdge[];
}

// Get graph with status in a single request (optimized endpoint)
// Use focus parameter to get only nodes related to a specific file
// Use hops parameter to control how many relationship levels to include
export async function getGraphWithStatus(
    sourceId: string,
    query?: GraphQuery
): Promise<GraphWithStatusResponse> {
    const params = new URLSearchParams();
    if (query?.folder) params.set('folder', query.folder);
    if (query?.focus) params.set('focus', query.focus);
    if (query?.hops) params.set('hops', query.hops.toString());

    const url = `${API_BASE}/api/sources/${sourceId}/graph/with_status${
        params.toString() ? '?' + params.toString() : ''
    }`;
    const response = await fetch(url);
    if (!response.ok) {
        throw new Error(`Failed to get graph with status: ${response.statusText}`);
    }
    return response.json();
}



export interface RenameNoteRequest {
    old_path: string;
    new_path: string;
}

export async function renameNote(sourceId: string, oldPath: string, newPath: string): Promise<void> {
    const response = await fetch(`${API_BASE}/api/sources/${sourceId}/notes/rename`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify({ old_path: oldPath, new_path: newPath }),
    });

    if (!response.ok) {
        throw new Error(`Failed to rename note: ${response.statusText}`);
    }
}
