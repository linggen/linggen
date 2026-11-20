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
