// What her bubble carries beside her words, read off one `pet_speak` cue:
// the problem her line raised with its offers (`lib/petOffers.mts`), and —
// for a notice that a run waits on the person — the waiting question, whose
// own card the bubble shows (`PendingAskCard`, the card the page shows too).

export type CueOffers<O> = { problem: string; offers: O[] };
export type SpeechAbout<O> = { raised?: CueOffers<O>; questionId?: string };

/** The cue's extras, or none when it carries nothing but words. */
export function speechAbout<O>(data: { problem?: string | null; offers?: O[] | null; question_id?: string | null } | null | undefined): SpeechAbout<O> | undefined {
  const raised = data?.problem ? { problem: data.problem, offers: data.offers ?? [] } : undefined;
  const questionId = data?.question_id || undefined;
  if (!raised && !questionId) return undefined;
  return { ...(raised ? { raised } : {}), ...(questionId ? { questionId } : {}) };
}

/** Her speech once question `widgetId` is answered anywhere: its card goes,
 *  her words stay. Unchanged for any other question. */
export function withoutAnswered<S extends { questionId?: string }>(speech: S | null, widgetId: string): S | null {
  if (!speech || speech.questionId !== widgetId) return speech;
  const { questionId: _gone, ...rest } = speech;
  void _gone;
  return rest as S;
}

/** Whether a line of hers is voiced on this surface: her cue says so (it is not
 *  muted, and the person cannot see the line where it shows) and the person is
 *  not speaking to her this moment (a hold to talk is on, lib/yinyueTalk.ts) —
 *  then the words show and no audio is made. The page's one rule for sound. */
export function voicedHere(cueVoice: unknown, personTalking: boolean): boolean {
  return cueVoice !== false && !personTalking;
}
