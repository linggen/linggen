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
