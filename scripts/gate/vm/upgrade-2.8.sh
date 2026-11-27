# 2.7.0 → 2.8.0: only in the scratch upgrade VM.
upgrade_28_applies() { [ "$PREV_LING" = 2.7.0 ] && [ "$LING_VERSION" = 2.8.0 ]; }

upgrade_28_seed() {
  local config="$LG/config/linggen.runtime.toml"
  [ -s "$config" ] && printf '\n%s\n' \
    '[[models]]' 'id = "gate-legacy-price"' 'provider = "openai"' \
    "url = \"$MODEL/v1\"" 'model = "gate-legacy-wire-id"' \
    'price = { input = 1.0, output = 2.0 }' \
    'price_lookup = { checked_on = "2026-01-01" }' \
    '[model_catalog]' 'edition = "mainland"' \
    '[spend_caps]' 'gate-legacy = 1.0' >>"$config"
  check "2.8 upgrade: seed retired 2.7 settings" $? "cannot seed $config"
}

upgrade_28_get() { curl -fsS -m 20 "http://127.0.0.1:$EP/api/$1" -o "$2"; }

upgrade_28_before() {
  # Published 2.7.0 reads the edition and price. The later 2.7-line keys
  # (price_lookup and spend_caps) are only present in the input TOML here.
  local config="$LG/config/linggen.runtime.toml"
  upgrade_28_get config "$OUT/config-2.7.json" &&
    jq -e '.model_catalog.edition == "mainland" and
      any(.models[]; .id == "gate-legacy-price" and .price.input == 1)' \
      "$OUT/config-2.7.json" >/dev/null &&
    grep -Fqx '[spend_caps]' "$config" && grep -q '^price_lookup = ' "$config"
  check "2.8 upgrade: legacy settings present in baseline" $? "edition/price missing from 2.7 API or lookup/cap missing from input TOML"
}

upgrade_28_models() {
  local missing
  missing="$(jq -cer --slurpfile before "$OUT/config-2.7.json" '
    def ids: [.models[].id];
    ($before[0] | ids) as $old | select(($old | length) > 0) | $old - ids
  ' "$1")" && [ "$missing" = '[]' ]
  check "2.8 upgrade: every 2.7 model row kept ($2)" $? "missing ids: ${missing:-no valid model snapshot}" \
    "$(jq '.models | length' "$OUT/config-2.7.json") baseline model IDs retained"
}

upgrade_28_removed() {
  local config="$OUT/config-2.8.json" saved="$LG/config/linggen.runtime.toml" rc
  local retired='model_catalog|spend_caps|price|price_lookup'
  upgrade_28_get config "$config"
  check "2.8 upgrade: read upgraded config" $? "GET /api/config did not succeed"
  upgrade_28_models "$config" loaded
  jq -e '(.models | type == "array") and ([.. | objects | keys[]] |
    all(. != "model_catalog" and . != "spend_caps" and . != "price" and . != "price_lookup"))' "$config" >/dev/null
  check "2.8 upgrade: no curated edition, prices or spend caps in active config" $? "retired settings present or /api/config invalid"
  # The legacy TOML can remain until the first settings save. Exercise that
  # save and check what actually persisted, not just what the API projects.
  curl -fsS -m 30 -H 'content-type: application/json' --data-binary "@$config" \
    "http://127.0.0.1:$EP/api/config" -o "$OUT/config-save-2.8.json" &&
    jq -e '.status == "ok"' "$OUT/config-save-2.8.json" >/dev/null && [ -s "$saved" ]
  rc=$?
  if [ "$rc" = 0 ]; then
    grep -Eq "^[[:space:]]*(\\[+($retired)([.]|\\])|($retired)[[:space:]]*=)" "$saved"
    [ "$?" = 1 ]
    rc=$?
  fi
  check "2.8 upgrade: saved TOML drops curated edition, prices and caps" "$rc" "settings save failed or retired keys remain"
  upgrade_28_get config "$OUT/config-saved-2.8.json"
  upgrade_28_models "$OUT/config-saved-2.8.json" saved

  # The 2.7 engine hard-codes the HTTPS path, outside the release mirror.
  # Check the actual release binary for that fetch target and cache marker;
  # a quiet mirror alone would not distinguish removal from a dormant fetch.
  LC_ALL=C grep -aqE 'model-catalog/v2|model-catalog-v2' "$LING"
  [ "$?" = 1 ]
  check "2.8 upgrade: no curated v2 fetch target or cache in engine" $? "legacy catalog marker found or binary unreadable"
  upgrade_28_get uses "$OUT/uses-2.8.json" && jq -e '
    (.speak.options | type == "array" and length > 0) and
    ([.speak.options[] | .id, .name] | all(ascii_downcase | contains("kokoro") | not)) and
    .speak.effective != "kokoro"' "$OUT/uses-2.8.json" >/dev/null
  check "2.8 upgrade: no Kokoro option in /api/uses" $? "Kokoro offered or speech options missing"
  curl -fsS -m 30 -H 'content-type: application/json' -d '{"text":"Release gate."}' \
    "http://127.0.0.1:$EP/api/tts" -o "$OUT/tts-2.8.wav" &&
    [ "$(head -c 4 "$OUT/tts-2.8.wav")" = RIFF ]
  check "2.8 upgrade: read-aloud still returns audio" $? "no WAV returned"
  LC_ALL=C grep -aqiE 'kokoro-voices|hexgrad/Kokoro-82M' "$LING"
  rc=$?
  [ "$rc" = 1 ] && [ ! -e "$LG/models/kokoro-voices" ]
  check "2.8 upgrade: no Kokoro voice fetch target or downloaded voices" $? "legacy fetch marker or Kokoro voice directory found (or binary unreadable)"
}
