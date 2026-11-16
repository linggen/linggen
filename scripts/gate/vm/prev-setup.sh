# Release gate — make the old-user VM (linggen-prev): the PUBLISHED releases
# installed the normal way (no mirror), then a fixtures home on top: sessions,
# config, skills, a mission, and a ling-mem store written by the old binary.
# (The takeover rows — long-term, staged, core — are written by upgrade.sh
# through the running old daemon, so every baseline gets them.) Published
# 2.3 on, memory is inside the engine: the store is written through it.
# Env from the host: MODEL (the fake model tunnel). Files: ~/gate/home (the
# rendered tests/fixtures/home), ~/gate/alex.json.
. ./common.sh

curl -fsSL https://linggen.dev/install.sh | bash >"$OUT/install.log" 2>&1
check "prev: install.sh (published)" $? "$(tail -3 "$OUT/install.log")" "ling $(ver_of "$(ling_bin)"), ling-mem $(ver_of "$(mem_bin)")"
curl -fsSL https://linggen.dev/install-app.sh | bash -s -- --no-launch >"$OUT/install-app.log" 2>&1
check "prev: install-app.sh (published)" $? "$(tail -3 "$OUT/install-app.log")" "Linggen.app $(app_version)"

# First run of the old engine, so its own state exists (db, built-in missions).
start_engine 90
check "prev: old engine first run" $? "no health"
is_standin "$(mem_bin)" || mem_health 60 || start_mem 60
stop_engine

# The fixtures home over it. The old engine's own built-in-mission seed stays.
cp -R "$OUT/home/." "$LG/"
info "prev: fixtures home" "$(ls "$LG/sessions" | wc -l | tr -d ' ') sessions, skills: $(ls "$LG/skills" | tr '\n' ' ')"

# A memory store written by the old ling-mem (needs its embedder: the first
# add downloads the model, ~1.2 GB).
if is_standin "$(mem_bin)"; then start_engine 90; else start_mem 60; fi
jq -c '.facts[]' "$OUT/alex.json" | sed "s#{{WORK}}#$HOME/work#g" >"$OUT/facts.ndjson"
mkdir -p "$HOME/work"
"$(mem_bin)" add --stdin <"$OUT/facts.ndjson" >"$OUT/mem-add.log" 2>&1
check "prev: old ling-mem stores the fixture facts" $? "$(tail -3 "$OUT/mem-add.log")"
"$(mem_bin)" stats >"$OUT/stats-prev.json" 2>&1
info "prev: store" "$(jq -c '{total, schema_version, per_tier}' "$OUT/stats-prev.json" 2>/dev/null | head -c 200)"

# The previous release's Claude Code plugin, as a user has it: 2.2.x took
# linggen@linggen-memory from the linggen-memory repo (its last release
# carries the same marketplace as a bundle).
OLD_BUNDLE=https://github.com/linggen/linggen-memory/releases/latest/download/linggen-plugin.tar.gz
OLD_MARKETPLACE="$HOME/.gate-old-plugin/marketplace"
if curl -fsSL https://claude.ai/install.sh | bash >"$OUT/claude-install.log" 2>&1 && command -v claude >/dev/null; then
  # The GitHub marketplace needs git, and a clean Mac's git is the Command
  # Line Tools stub (an install dialog) — then use that release's bundle.
  if claude plugin marketplace add linggen/linggen-memory >"$OUT/claude-plugin.log" 2>&1; then
    claude plugin install linggen@linggen-memory >>"$OUT/claude-plugin.log" 2>&1
    check "prev: claude plugin (published)" $? "$(tail -3 "$OUT/claude-plugin.log")"
  else
    warn "prev: claude plugin (published)" "GitHub marketplace needs git (CLT dialog on a clean Mac); using linggen-memory's release bundle"
    # Outside ~/gate: the host empties that folder before every VM script, and
    # the old plugin must still load when upgrade.sh looks at it.
    mkdir -p "$OLD_MARKETPLACE"
    curl -fsSL "$OLD_BUNDLE" | tar -xzf - -C "$OLD_MARKETPLACE" \
      && claude plugin marketplace add "$OLD_MARKETPLACE" >>"$OUT/claude-plugin.log" 2>&1 \
      && claude plugin install linggen@linggen-memory >>"$OUT/claude-plugin.log" 2>&1
    check "prev: claude plugin (release bundle)" $? "$(tail -3 "$OUT/claude-plugin.log")"
  fi
else
  gap "prev: claude plugin (published)" "claude CLI did not install"
fi

stop_engine
stop_mem
exit 0
