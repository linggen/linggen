import type { Env, TrackEventPayload, TrackResponse, ErrorResponse, SkillInstallPayload, SkillInstallResponse } from './types';

/**
 * Linggen Analytics & Skills Registry Worker
 *
 * Receives anonymous usage events and skill install records.
 *
 * Endpoints:
 *   POST /track - Record an analytics event
 *   POST /skills/install - Record a skill install
 *   GET /skills - List all skills (paginated, sorted by install_count)
 *   GET /skills/search - Search skills by name, url, or content
 *   GET /health - Health check endpoint
 */

// CORS headers for cross-origin requests (if needed)
const corsHeaders = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, X-API-Key',
};

function getApiKey(env: Env): string | undefined {
  return env.API_KEY;
}

function jsonResponse(body: unknown, init: ResponseInit): Response {
  return new Response(JSON.stringify(body), {
    ...init,
    headers: { ...corsHeaders, ...(init.headers || {}), 'Content-Type': 'application/json' },
  });
}

function requireApiKey(request: Request, env: Env): Response | null {
  const expected = getApiKey(env);
  if (!expected) {
    const error: ErrorResponse = {
      success: false,
      error: 'Server misconfigured: API_KEY is not set',
    };
    return jsonResponse(error, { status: 500 });
  }

  const apiKey = request.headers.get('X-API-Key');
  if (apiKey !== expected) {
    const error: ErrorResponse = { success: false, error: 'Unauthorized' };
    return jsonResponse(error, { status: 401 });
  }

  return null;
}

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
  const auth = requireApiKey(request, env);
  if (auth) return auth;

  // Parse request body
  let data: unknown;
  try {
    data = await request.json();
  } catch {
    const error: ErrorResponse = { success: false, error: 'Invalid JSON body' };
    return jsonResponse(error, { status: 400 });
  }

  // Validate payload
  const validation = validatePayload(data);
  if (!validation.valid) {
    const error: ErrorResponse = { success: false, error: validation.error };
    return jsonResponse(error, { status: 400 });
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

    return jsonResponse(response, { status: 200 });
  } catch (err) {
    console.error('D1 insert error:', err);
    const error: ErrorResponse = { success: false, error: 'Failed to record event' };
    return jsonResponse(error, { status: 500 });
  }
}

/**
 * Handle GET /health - Health check
 */
async function handleHealth(request: Request, env: Env): Promise<Response> {
  const auth = requireApiKey(request, env);
  if (auth) return auth;

  try {
    // Simple query to verify D1 connection
    await env.DB.prepare('SELECT 1').run();
    return jsonResponse({ status: 'ok', timestamp: new Date().toISOString() }, { status: 200 });
  } catch (err) {
    console.error('Health check failed:', err);
    return jsonResponse({ status: 'error', error: 'Database unavailable' }, { status: 503 });
  }
}

/**
 * Hash IP with salt for privacy
 */
async function hashIp(ip: string, salt: string): Promise<string> {
  const encoder = new TextEncoder();
  const data = encoder.encode(`${salt}:${ip}`);
  const hashBuffer = await crypto.subtle.digest('SHA-256', data);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Handle GET /skills - List all skills (Public)
 */
async function handleListSkills(request: Request, env: Env): Promise<Response> {
  const url = new URL(request.url);
  const page = Math.max(1, parseInt(url.searchParams.get('page') || '1'));
  const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get('limit') || '20')));
  const offset = (page - 1) * limit;

  try {
    // 1. Get total count
    const totalResult = await env.DB.prepare('SELECT COUNT(*) as total FROM skills').first<{ total: number }>();
    const total = totalResult?.total || 0;

    // 2. Get paginated results
    const { results } = await env.DB.prepare(
      'SELECT skill_id, url, skill, ref, content, install_count, updated_at FROM skills ORDER BY install_count DESC LIMIT ? OFFSET ?'
    )
      .bind(limit, offset)
      .all();

    return jsonResponse({
      success: true,
      skills: results,
      pagination: {
        total,
        page,
        limit,
        total_pages: Math.ceil(total / limit),
      }
    }, { status: 200 });
  } catch (err) {
    console.error('List skills error:', err);
    return jsonResponse({ success: false, error: 'Internal error' }, { status: 500 });
  }
}

/**
 * Handle GET /skills/search - Search skills by name, url, or content (Public)
 */
async function handleSearchSkills(request: Request, env: Env): Promise<Response> {
  const url = new URL(request.url);
  const query = url.searchParams.get('q')?.trim();

  if (!query) {
    return jsonResponse({ success: false, error: 'Missing search query parameter "q"' }, { status: 400 });
  }

  const page = Math.max(1, parseInt(url.searchParams.get('page') || '1'));
  const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get('limit') || '20')));
  const offset = (page - 1) * limit;

  try {
    // Create search pattern for LIKE queries
    const searchPattern = `%${query}%`;

    // 1. Get total count of matching skills
    const totalResult = await env.DB.prepare(
      'SELECT COUNT(*) as total FROM skills WHERE skill LIKE ? OR url LIKE ? OR content LIKE ?'
    )
      .bind(searchPattern, searchPattern, searchPattern)
      .first<{ total: number }>();
    const total = totalResult?.total || 0;

    // 2. Get paginated search results, ordered by install_count DESC
    const { results } = await env.DB.prepare(
      'SELECT skill_id, url, skill, ref, content, install_count, updated_at FROM skills WHERE skill LIKE ? OR url LIKE ? OR content LIKE ? ORDER BY install_count DESC LIMIT ? OFFSET ?'
    )
      .bind(searchPattern, searchPattern, searchPattern, limit, offset)
      .all();

    return jsonResponse({
      success: true,
      query,
      skills: results,
      pagination: {
        total,
        page,
        limit,
        total_pages: Math.ceil(total / limit),
      }
    }, { status: 200 });
  } catch (err) {
    console.error('Search skills error:', err);
    return jsonResponse({ success: false, error: 'Internal error' }, { status: 500 });
  }
}

/**
 * Handle POST /skills/install - Record a skill install
 */
async function handleSkillInstall(request: Request, env: Env): Promise<Response> {
  const auth = requireApiKey(request, env);
  if (auth) return auth;

  // 2. Parse body
  let payload: SkillInstallPayload;
  try {
    payload = await request.json();
  } catch {
    return jsonResponse({ success: false, error: 'Invalid JSON' }, { status: 400 });
  }

  const { url, skill, ref, content } = payload;
  if (!url || !skill) {
    return jsonResponse({ success: false, error: 'Missing url or skill' }, { status: 400 });
  }

  const skill_id = `${url}/${skill}`;
  const ip = request.headers.get('CF-Connecting-IP') || '127.0.0.1';
  const salt = env.IP_HASH_SALT || 'default-salt-change-me';
  const ip_hash = await hashIp(ip, salt);

  // 1 hour cooldown bucket
  const bucket = Math.floor(Date.now() / 1000 / 3600);

  // Simple IP-based rate limiting (max 100 requests per hour)
  try {
    const { count } = await env.DB.prepare(
      'SELECT COUNT(*) as count FROM skill_installs WHERE ip_hash = ? AND bucket = ?'
    )
      .bind(ip_hash, bucket)
      .first<{ count: number }>() || { count: 0 };

    if (count > 100) {
      return jsonResponse({ success: false, error: 'Rate limit exceeded' }, { status: 429 });
    }
  } catch (err) {
    console.error('Rate limit check error:', err);
  }

  try {
    // 3. Try to record the install (deduped by UNIQUE index on skill_id, ip_hash, bucket)
    const installResult = await env.DB.prepare(
      `INSERT OR IGNORE INTO skill_installs (skill_id, ip_hash, bucket, created_at)
       VALUES (?, ?, ?, ?)`
    )
      .bind(skill_id, ip_hash, bucket, new Date().toISOString())
      .run();

    const counted = installResult.meta.changes > 0;

    if (counted) {
      // 4. Update global count and content
      await env.DB.prepare(
        `INSERT INTO skills (skill_id, url, skill, ref, content, install_count, updated_at)
         VALUES (?, ?, ?, ?, ?, 1, ?)
         ON CONFLICT(skill_id) DO UPDATE SET
           install_count = install_count + 1,
           ref = EXCLUDED.ref,
           content = COALESCE(EXCLUDED.content, skills.content),
           updated_at = EXCLUDED.updated_at`
      )
        .bind(skill_id, url, skill, ref, content || null, new Date().toISOString())
        .run();
    } else {
      // Just update content and ref even if count isn't incremented (cooldown)
      await env.DB.prepare(
        `UPDATE skills SET 
           ref = ?, 
           content = COALESCE(?, content),
           updated_at = ?
         WHERE skill_id = ?`
      )
        .bind(ref, content || null, new Date().toISOString(), skill_id)
        .run();
    }

    const response: SkillInstallResponse = {
      ok: true,
      counted,
      skill_id,
      cooldown_seconds: 3600,
    };

    return jsonResponse(response, { status: 200 });
  } catch (err) {
    console.error('Skill install error:', err);
    return jsonResponse({ success: false, error: 'Internal error' }, { status: 500 });
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

    if (url.pathname === '/skills/install' && method === 'POST') {
      return handleSkillInstall(request, env);
    }

    if (url.pathname === '/skills' && method === 'GET') {
      return handleListSkills(request, env);
    }

    if (url.pathname === '/skills/search' && method === 'GET') {
      return handleSearchSkills(request, env);
    }

    if (url.pathname === '/health' && method === 'GET') {
      return handleHealth(request, env);
    }

    // 404 for unknown routes
    return jsonResponse({ error: 'Not found' }, { status: 404 });
  },
};
