/**
 * Event types tracked by Linggen analytics
 */
export type EventType = 'app_started' | 'source_added';

/**
 * Supported platforms
 */
export type Platform = 'macos' | 'windows' | 'linux' | 'unknown';

/**
 * Source types for source_added events
 */
export type SourceType = 'local' | 'git' | 'web' | 'uploads';

/**
 * Size bucket for project size classification
 */
export type SizeBucket = 'small' | 'medium' | 'large' | 'xlarge';

/**
 * Base payload for all events
 */
export interface BaseEventPayload {
  installation_id: string;
  event_type: EventType;
  app_version: string;
  platform: Platform;
  timestamp?: string; // ISO 8601, server will use current time if not provided
}

/**
 * Payload for app_started event
 */
export interface AppStartedPayload extends BaseEventPayload {
  event_type: 'app_started';
  payload?: {
    first_launch?: boolean;
  };
}

/**
 * Payload for source_added event
 */
export interface SourceAddedPayload extends BaseEventPayload {
  event_type: 'source_added';
  payload: {
    source_type: SourceType;
    size_bucket?: SizeBucket; // small: <100 files, medium: 100-1000, large: 1000-10000, xlarge: >10000
    file_count?: number;
  };
}

/**
 * Union type for all event payloads
 */
export type TrackEventPayload = AppStartedPayload | SourceAddedPayload;

/**
 * API response for successful tracking
 */
export interface TrackResponse {
  success: boolean;
  event_id?: string;
}

/**
 * API error response
 */
export interface ErrorResponse {
  success: false;
  error: string;
}

/**
 * D1 database environment binding
 */
export interface Env {
  DB: D1Database;
  ANALYTICS_API_KEY?: string;
}
