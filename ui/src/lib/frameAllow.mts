// The `allow` attribute for a skill app's frame (skill-spec § App skills,
// `app.allow`). A same-origin frame inherits camera and microphone from the page by default,
// so a feature the app did not declare is denied outright ('none'), not just
// left out. Mirrors the engine's `DEVICE_FEATURES` and its Permissions-Policy.
export const DEVICE_FEATURES = ['camera', 'microphone'] as const;

export function frameAllow(declared?: readonly string[] | null): string {
  return DEVICE_FEATURES
    .map((f) => (declared?.includes(f) ? f : `${f} 'none'`))
    .join('; ');
}
