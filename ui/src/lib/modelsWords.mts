/**
 * Every fixed word of Settings → Models, in one list. A component never writes
 * a label of its own: it says `W.x`, or `say(W.x, {slot})` for one with a
 * `{slot}`. ui/tests/modelsWords.test.mts holds the list to the rules — each
 * has its 简体中文 in strings-zh.ts, and none is longer than twenty words — so a
 * new label cannot ship half-said. A sentence is one string (the page is put
 * into the person's language one text node at a time, lib/i18n.ts).
 */
export const W = {
  // ── The page ──
  defaultModel: 'Default model',
  defaultModelHint: 'New chats start on this model. A chat can switch its own.',
  chat: 'Chat',
  notSet: 'Not set',
  liveVoice: 'Live voice',
  readAloud: 'Read aloud',
  dictation: 'Dictation',
  drawing: 'Drawing',
  ready: 'Ready',
  notReady: 'Not ready',
  notInstalled: 'Not installed',
  notDownloaded: 'Not downloaded',
  gb: '{n} GB',
  providers: 'Providers',
  providersHint: 'Each row is one connection: who bills you, or where it runs.',
  loading: 'Loading…',
  providersFailed: 'The providers could not be loaded.',
  retry: 'Retry',
  // ── The two groups ──
  readyToUse: 'Ready to use',
  readyToUseHint: 'Sign in; no key needed.',
  ownKeys: 'Your own keys and this Mac',
  ownKeysHint: 'Pay the provider directly, or run free on this Mac.',
  kindAccount: 'Account',
  kindSubscription: 'Subscription',
  kindKey: 'Key',
  kindLocal: 'This Mac',
  thisMac: 'This Mac',
  // ── Sign-in rows ──
  signedIn: 'Signed in',
  notSignedIn: 'Not signed in',
  signInLinggen: 'Sign in to linggen.dev',
  signInChatgpt: 'Sign in with ChatGPT',
  signOut: 'Sign out',
  waitingBrowser: 'Waiting for browser login...',
  cloudNote: 'Uses your linggen.dev account.',
  chatgptNote: 'Uses your ChatGPT subscription; no key needed.',
  // ── A key provider ──
  keyLabel: 'Key',
  keySet: 'Key set',
  pasteKey: 'Paste your key',
  saveKey: 'Save key',
  replaceKey: 'Replace',
  removeKey: 'Remove',
  shareKey: 'Models on this row share this key',
  noKeyYet: 'No key yet',
  nothingTicked: 'Nothing ticked',
  models: 'Models',
  noModelsYet: 'No models yet. Paste a key, or add one under Advanced.',
  // ── A family line ──
  now: 'Now {name}',
  wouldBe: 'Would be {name}',
  priceIn: 'input ${input} / output ${output} per million',
  noPrice: 'No price',
  estimate: 'estimate',
  pinned: 'Pinned',
  usedDefault: 'Default chat',
  usedFallback: 'Fallback',
  usedCompanion: 'Companion',
  usedAgent: 'Agent {who}',
  liveTag: 'Live voice',
  mergeRows: 'Merge',
  holdChoose: 'Pick a version',
  checkingKey: 'Checking this key…',
  listFailed: 'Could not connect. Try again later.',
  chooseVersion: 'Choose a version',
  options: 'Options',
  pinVersion: 'Pin to this version',
  makeDefault: 'Make default',
  followAgain: 'Follow automatically again',
  removeModel: 'Remove',
  followNote: 'New generations are followed automatically, with one notice.',
  moreN: 'More ({n})',
  searchModels: 'Search models',
  totalN: '{n} in all',
  searchToSee: 'Search to see more',
  // ── The move notice ──
  movedLine: 'Moved from {from} to {to}',
  wentBackLine: 'Went back from {from} to {to}',
  gotIt: 'Got it',
  switchBack: 'Switch back',
  // ── The price footer ──
  estimatesFrom: 'Prices are estimates, from the list of {date}.',
  listOld: 'The price list is {n} days old.',
  // ── Advanced ──
  advanced: 'Advanced',
  notInList: 'A model that is not in the list',
  modelName: 'Model name',
  add: 'Add',
  addedByName: 'Added by name',
  address: 'Address',
  saveAddress: 'Save address',
  addressNote: 'Changing it moves the key and the models with it.',
  // ── Add a provider ──
  addProvider: 'Add provider',
  pickProvider: 'Pick a provider',
  close: 'Close',
  // ── What went wrong (a route's fixed answer, as a short fixed label) ──
  failNotAccepted: 'That was not accepted.',
  failNotFound: 'Not found. Reload the page.',
  failNotSaved: 'Could not save.',
  failInUse: 'Already in use.',
  failFirstDefault: 'Pick another default model first.',
  failNoKey: 'Add a key first.',
  failNeedsChoice: 'Choose a version first.',
  failNoAccess: 'This key cannot use it.',
  failNoPrevious: 'The earlier version is gone.',
  failOwnerOnly: 'Only the owner can change this.',
  failOther: 'Something went wrong.',
} as const;

export type Word = (typeof W)[keyof typeof W];

/** A word with its `{slot}`s filled. */
export function say(word: Word, slots: Record<string, string | number>): string {
  return word.replace(/\{(\w+)\}/g, (hole, name: string) => (name in slots ? String(slots[name]) : hole));
}
