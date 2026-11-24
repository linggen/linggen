import { translate } from '../../lib/i18n';
import type { UseOption } from '../../lib/modelUses';
import { W, say } from '../../lib/modelsWords.mts';
import { ConnectionTag, connectionWord } from './shell';

const localized = (text: string) => translate(text) ?? text;

/** Names and source facts come from /api/uses; only their presentation lives here. */
export const modelName = (option: UseOption) => localized(option.name);

export function sourceText({ source }: UseOption) {
  const size = source.size_gb == null ? null : say(W.gb, { n: source.size_gb });
  return [source.provider ?? connectionWord(source.way), size, source.free ? W.freeTag : null]
    .filter((part): part is string => part != null).map(localized).join(' · ');
}

// Native option elements cannot render styled tags. Keep the same label and facts.
export const nativeModelLabel = (option: UseOption) => `${modelName(option)} · ${sourceText(option)}`;

export function ModelSourceTag({ option }: { option: UseOption }) {
  return <ConnectionTag way={option.source.way}><span data-no-i18n>{sourceText(option)}</span></ConnectionTag>;
}
