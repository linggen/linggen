import type { Env, TrackEventPayload, TrackResponse, ErrorResponse } from './types';

/**
 * Linggen Analytics Worker
 * 
 * Receives anonymous usage events from the Linggen desktop app and stores them in D1.
 * 
 * Endpoints:
 *   POST /track - Record an analytics event
 *   GET /health - Health check endpoint
 */

// CORS headers for cross-origin requests (if needed)
const corsHeaders = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, X-API-Key',
};

/**
 * Validate the incoming event payload
 */
function validatePayload(data: unknown): { valid: true; payload: TrackEventPayload } | { valid: false; error: string } {
  if (!data || typeof data !== 'object') {
    return { valid: false, error: 'Invalid request body' };
  }

  const payload = data as Record<string, unknown>;

  // Required fields
  if (!payload.installation_id || typeof payload.installation_id !== 'string') {
    return { valid: false, error: 'Missing or invalid installation_id' };
  }

  if (!payload.event_type || typeof payload.event_type !== 'string') {
    return { valid: false, error: 'Missing or invalid event_type' };
  }

  const validEventTypes = ['app_started', 'source_added'];
  if (!validEventTypes.includes(payload.event_type)) {
    return { valid: false, error: `Invalid event_type: ${payload.event_type}` };
  }

  if (!payload.app_version || typeof payload.app_version !== 'string') {
    return { valid: false, error: 'Missing or invalid app_version' };
  }

  if (!payload.platform || typeof payload.platform !== 'string') {
    return { valid: false, error: 'Missing or invalid platform' };
  }

  const validPlatforms = ['macos', 'windows', 'linux', 'unknown'];
  if (!validPlatforms.includes(payload.platform)) {
    return { valid: false, error: `Invalid platform: ${payload.platform}` };
  }

  // Validate source_added specific fields
  if (payload.event_type === 'source_added') {
    if (!payload.payload || typeof payload.payload !== 'object') {
      return { valid: false, error: 'source_added event requires payload object' };
    }
    const eventPayload = payload.payload as Record<string, unknown>;
    if (!eventPayload.source_type || typeof eventPayload.source_type !== 'string') {
      return { valid: false, error: 'source_added event requires payload.source_type' };
    }
    const validSourceTypes = ['local', 'git', 'web', 'uploads'];
    if (!validSourceTypes.includes(eventPayload.source_type)) {
      return { valid: false, error: `Invalid source_type: ${eventPayload.source_type}` };
    }
  }

  return { valid: true, payload: payload as unknown as TrackEventPayload };
}

/**
 * Handle POST /track - Record an analytics event
 */
async function handleTrack(request: Request, env: Env): Promise<Response> {
  // Optional: Check API key if configured
  if (env.ANALYTICS_API_KEY) {
    const apiKey = request.headers.get('X-API-Key');
    if (apiKey !== env.ANALYTICS_API_KEY) {
      const error: ErrorResponse = { success: false, error: 'Unauthorized' };
      return new Response(JSON.stringify(error), {
        status: 401,
        headers: { ...corsHeaders, 'Content-Type': 'application/json' },
      });
    }
  }

  // Parse request body
  let data: unknown;
  try {
    data = await request.json();
  } catch {
    const error: ErrorResponse = { success: false, error: 'Invalid JSON body' };
    return new Response(JSON.stringify(error), {
      status: 400,
      headers: { ...corsHeaders, 'Content-Type': 'application/json' },
    });
  }

  // Validate payload
  const validation = validatePayload(data);
  if (!validation.valid) {
    const error: ErrorResponse = { success: false, error: validation.error };
    return new Response(JSON.stringify(error), {
      status: 400,
      headers: { ...corsHeaders, 'Content-Type': 'application/json' },
    });
  }

  const payload = validation.payload;
  const timestamp = payload.timestamp || new Date().toISOString();
  const eventPayload = 'payload' in payload ? JSON.stringify(payload.payload) : null;

  // Insert into D1
  try {
    const result = await env.DB.prepare(
      `INSERT INTO events (installation_id, event_type, app_version, platform, payload, created_at)
       VALUES (?, ?, ?, ?, ?, ?)`
    )
      .bind(
        payload.installation_id,
        payload.event_type,
        payload.app_version,
        payload.platform,
        eventPayload,
        timestamp
      )
      .run();

    const response: TrackResponse = {
      success: true,
      event_id: result.meta.last_row_id?.toString(),
    };

    return new Response(JSON.stringify(response), {
      status: 200,
      headers: { ...corsHeaders, 'Content-Type': 'application/json' },
    });
  } catch (err) {
    console.error('D1 insert error:', err);
    const error: ErrorResponse = { success: false, error: 'Failed to record event' };
    return new Response(JSON.stringify(error), {
      status: 500,
      headers: { ...corsHeaders, 'Content-Type': 'application/json' },
    });
  }
}

/**
 * Handle GET /health - Health check
 */
async function handleHealth(env: Env): Promise<Response> {
  try {
    // Simple query to verify D1 connection
    await env.DB.prepare('SELECT 1').run();
    return new Response(JSON.stringify({ status: 'ok', timestamp: new Date().toISOString() }), {
      status: 200,
      headers: { ...corsHeaders, 'Content-Type': 'application/json' },
    });
  } catch (err) {
    console.error('Health check failed:', err);
    return new Response(JSON.stringify({ status: 'error', error: 'Database unavailable' }), {
      status: 503,
      headers: { ...corsHeaders, 'Content-Type': 'application/json' },
    });
  }
}

/**
 * Handle OPTIONS requests for CORS preflight
 */
function handleOptions(): Response {
  return new Response(null, {
    status: 204,
    headers: corsHeaders,
  });
}

/**
 * Main fetch handler
 */
export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const method = request.method;

    // Handle CORS preflight
    if (method === 'OPTIONS') {
      return handleOptions();
    }

    // Route requests
    if (url.pathname === '/track' && method === 'POST') {
      return handleTrack(request, env);
    }

    if (url.pathname === '/health' && method === 'GET') {
      return handleHealth(env);
    }

    // 404 for unknown routes
    return new Response(JSON.stringify({ error: 'Not found' }), {
      status: 404,
      headers: { ...corsHeaders, 'Content-Type': 'application/json' },
    });
  },
};
