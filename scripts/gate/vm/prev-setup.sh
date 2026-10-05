# Release gate — make the old-user VM (linggen-prev): the PUBLISHED releases
# installed the normal way (no mirror), then a fixtures home on top: sessions,
# config, skills, a mission, and a ling-mem store written by the old binary.
# Env from the host: MODEL (the fake model tunnel). Files: ~/gate/home (the
# rendered tests/fixtures/home), ~/gate/alex.json, ~/gate/marketplace.
. ./common.sh

curl -fsSL https://linggen.dev/install.sh | bash >"$OUT/install.log" 2>&1
check "prev: install.sh (published)" $? "$(tail -3 "$OUT/install.log")" "ling $(ver_of "$(ling_bin)"), ling-mem $(ver_of "$(mem_bin)")"
curl -fsSL https://linggen.dev/install-app.sh | bash -s -- --no-launch >"$OUT/install-app.log" 2>&1
check "prev: install-app.sh (published)" $? "$(tail -3 "$OUT/install-app.log")" "Linggen.app $(app_version)"

# First run of the old engine, so its own state exists (db, built-in missions).
start_engine 90
check "prev: old engine first run" $? "no health"
mem_health 60 || start_mem 60
stop_engine

# The fixtures home over it. The old engine's own built-in-mission seed stays.
cp -R "$OUT/home/." "$LG/"
info "prev: fixtures home" "$(ls "$LG/sessions" | wc -l | tr -d ' ') sessions, skills: $(ls "$LG/skills" | tr '\n' ' ')"

# A memory store written by the old ling-mem (needs its embedder: the first
# add downloads the model, ~1.2 GB).
start_mem 60
jq -c '.facts[]' "$OUT/alex.json" | sed "s#{{WORK}}#$HOME/work#g" >"$OUT/facts.ndjson"
mkdir -p "$HOME/work"
"$(mem_bin)" add --stdin <"$OUT/facts.ndjson" >"$OUT/mem-add.log" 2>&1
check "prev: old ling-mem stores the fixture facts" $? "$(tail -3 "$OUT/mem-add.log")"
"$(mem_bin)" stats >"$OUT/stats-prev.json" 2>&1
info "prev: store" "$(jq -c '{total, schema_version, per_tier}' "$OUT/stats-prev.json" 2>/dev/null | head -c 200)"

# The published Claude Code plugin (GitHub marketplace), as a user has it.
if curl -fsSL https://claude.ai/install.sh | bash >"$OUT/claude-install.log" 2>&1 && command -v claude >/dev/null; then
  # The GitHub marketplace needs git, and a clean Mac's git is the Command
  # Line Tools stub (an install dialog) — then use the gate's copy instead.
  if claude plugin marketplace add linggen/linggen-memory >"$OUT/claude-plugin.log" 2>&1; then
    claude plugin install linggen@linggen-memory >>"$OUT/claude-plugin.log" 2>&1
    check "prev: claude plugin (published)" $? "$(tail -3 "$OUT/claude-plugin.log")"
  else
    warn "prev: claude plugin (published)" "GitHub marketplace needs git (CLT dialog on a clean Mac); using the gate's copy"
    claude plugin marketplace add "$OUT/marketplace" >>"$OUT/claude-plugin.log" 2>&1 \
      && claude plugin install linggen@linggen-memory >>"$OUT/claude-plugin.log" 2>&1
    check "prev: claude plugin (gate copy)" $? "$(tail -3 "$OUT/claude-plugin.log")"
  fi
else
  gap "prev: claude plugin (published)" "claude CLI did not install"
fi

stop_engine
stop_mem
exit 0
