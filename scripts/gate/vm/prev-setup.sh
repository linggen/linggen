# Release gate — make the old-user VM (linggen-prev): the PUBLISHED releases
# installed the normal way (no mirror), then a fixtures home on top: sessions,
# config, skills, a mission, and a memory store written by the old release.
# (The takeover / in-place rows — long-term, staged, core — are written by
# upgrade.sh through what serves memory there, so every baseline gets them.)
# Two kinds, decided by what install.sh laid down: a `ling` that hosts memory
# (published 2.3.0 on — no ling-mem to install: the store is written through
# `ling mem`, the plugin is that release's linggen@linggen), or ≤ 2.2.x with
# its ling-mem daemon and linggen@linggen-memory.
# Env from the host: MODEL (the fake model tunnel). Files: ~/gate/home (the
# rendered tests/fixtures/home), ~/gate/alex.json.
. ./common.sh

# Use the new installer to fetch the previous *published* app, not the draft.
# The old frozen VM with its standalone ling-mem is kept separately (--prev).
curl -fsSL "$SITE/install.sh" | env -u LINGGEN_RELEASE_BASE bash >"$OUT/prev-install.log" 2>&1
check "prev: install.sh (published assets)" $? "$(tail -3 "$OUT/prev-install.log")" "Linggen.app $(app_version)"
one_ling "prev: published app"

HOSTS_MEMORY=false
"$(ling_bin)" mem --help >/dev/null 2>&1 && HOSTS_MEMORY=true
info "prev: kind" "$($HOSTS_MEMORY && echo "ling $(ver_of "$(ling_bin)") hosts memory" || echo "ling $(ver_of "$(ling_bin)") with its ling-mem daemon")"

# First run of the old engine, so its own state exists (db, built-in missions).
start_engine 90
check "prev: old engine first run" $? "no health"
$HOSTS_MEMORY || is_standin "$(mem_bin)" || mem_health 60 || start_mem 60
stop_engine

# The fixtures home over it. The old engine's own built-in-mission seed stays.
cp -R "$OUT/home/." "$LG/"
info "prev: fixtures home" "$(ls "$LG/sessions" | wc -l | tr -d ' ') sessions, skills: $(ls "$LG/skills" | tr '\n' ' ')"

# A memory store written by the old release (needs its embedder: the first
# add downloads the model, ~1.2 GB).
jq -c '.facts[]' "$OUT/alex.json" | sed "s#{{WORK}}#$HOME/work#g" >"$OUT/facts.ndjson"
mkdir -p "$HOME/work"
if $HOSTS_MEMORY; then
  start_engine 90
  engine_mem_health 60
  check "prev: the engine serves memory (/mem/api/health)" $? "no ok from $ENGINE_MEM/api/health"
  # From 2.4 the embedder warms up in the background — on a clean VM that
  # includes the model's first download — and a call waits at most 20 s for
  # it, then is refused WARMING_UP. Seed only once a search answers.
  for _ in $(seq 60); do
    "$(ling_bin)" mem search "warm-up probe" >"$OUT/mem-warm.log" 2>&1
    grep -q 'WARMING_UP' "$OUT/mem-warm.log" || break
  done
  info "prev: memory warm before seeding" "$(tail -c 160 "$OUT/mem-warm.log" | tr '\n' ' ')"
  "$(ling_bin)" mem add --stdin <"$OUT/facts.ndjson" >"$OUT/mem-add.log" 2>&1
  check "prev: ling mem stores the fixture facts" $? "$(tail -3 "$OUT/mem-add.log")"
  "$(ling_bin)" mem stats >"$OUT/stats-prev.json" 2>&1
else
  if is_standin "$(mem_bin)"; then start_engine 90; else start_mem 60; fi
  "$(mem_bin)" add --stdin <"$OUT/facts.ndjson" >"$OUT/mem-add.log" 2>&1
  check "prev: old ling-mem stores the fixture facts" $? "$(tail -3 "$OUT/mem-add.log")"
  "$(mem_bin)" stats >"$OUT/stats-prev.json" 2>&1
fi
info "prev: store" "$(jq -c '{total, schema_version, per_tier}' "$OUT/stats-prev.json" 2>/dev/null | head -c 200)"

# The previous release's Claude Code plugin, as a user has it: 2.3 on, the
# published install-plugin.sh (linggen@linggen, the engine release's bundle);
# 2.2.x took linggen@linggen-memory from the linggen-memory repo (its last
# release carries the same marketplace as a bundle).
OLD_BUNDLE=https://github.com/linggen/linggen-memory/releases/latest/download/linggen-plugin.tar.gz
OLD_MARKETPLACE="$HOME/.gate-old-plugin/marketplace"
if ! curl -fsSL https://claude.ai/install.sh | bash >"$OUT/claude-install.log" 2>&1 || ! command -v claude >/dev/null; then
  gap "prev: claude plugin (published)" "claude CLI did not install"
elif $HOSTS_MEMORY; then
  curl -fsSL https://linggen.dev/install-plugin.sh | bash -s -- --claude-only --bundle >"$OUT/claude-plugin.log" 2>&1
  check "prev: claude plugin (published install-plugin.sh)" $? "$(tail -3 "$OUT/claude-plugin.log")" "$(claude plugin list 2>/dev/null | grep -m1 -o 'linggen@[a-z-]*')"
else
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
fi

stop_engine
$HOSTS_MEMORY || stop_mem
exit 0
