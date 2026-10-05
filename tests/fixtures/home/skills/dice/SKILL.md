---
name: dice
description: A tiny dice table for the engine's system tests — Ling runs it, Yinyue sits beside the player.
allowed-tools: [AskUser]
user-invocable: true
cwd: ~/.linggen/skills/dice
members: [ling, yinyue]
place:
  yinyue:
    tools: [Peek]
    text: >-
      You sit at the dice table beside the player. Ling runs the table; you
      never roll for them.
permission:
  paths:
    - { path: ~/.linggen/skills/dice, mode: edit }
  warning: Dice keeps its table in its own folder.
tools:
  - name: Look
    description: The table as it stands, as JSON. Call it first.
    cmd: "sh $SKILL_DIR/scripts/look.sh"
    tier: read
    # What another member reads of it: the facts, never Ling's guide.
    others_read: [table.name, player.name, last_roll.value]
    timeout_ms: 5000
  - name: Roll
    description: Roll the die and keep the result on the table.
    cmd: "sh $SKILL_DIR/scripts/roll.sh {{faces}}"
    tier: edit
    timeout_ms: 5000
    args:
      faces:
        type: number
        required: true
        description: How many faces the die has.
  - name: Peek
    description: Yinyue's glance at the table — the last roll. Changes nothing.
    cmd: "sh $SKILL_DIR/scripts/look.sh"
    tier: read
    pet: true
    timeout_ms: 5000
---

You run the dice table. Call Look first, then answer the player in one line.
