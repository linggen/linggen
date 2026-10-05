---
name: table-check
description: Look at the dice table and say what was rolled last. A test mission, off.
schedule: '0 3 * * *'
enabled: false
agent: ling
kickoff:
  - "Look at the dice table and say the last roll."
allowed-tools: [Read]
---

Read the dice table and say the last roll in one line.
