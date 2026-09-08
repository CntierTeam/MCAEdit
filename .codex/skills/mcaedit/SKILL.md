---
name: mcaedit
description: >-
  Offline Minecraft Anvil (.mca) editor CLI for LLMs. Session-based edits with
  history undo/redo, multi-session collaboration, templates, and commit back to
  region/entities MCA files. Trigger on: MCAEdit, mcaedit, MCA editor, Anvil
  region edit, chunk palette, inspect select ASCII.
license: MIT
metadata:
  short-description: Offline MCA edit CLI (session → edit → commit)
---

# MCAEdit

Rust CLI binary: `mcaedit`.

## Hard rules

1. Always work inside a **session** (`--session <id>` or `MCAEDIT_SESSION`).
2. Edits touch the session work copy only; **`commit`** writes to the source world.
3. Prefer small `inspect select` boxes (≤16³) for LLM-readable palette + ASCII 3D.
4. Multi-session: one agent per session label; `sync` before editing after others commit.
5. `minecraft:void_air` in `set-section` means **keep existing** (skip cell).

## Common workflow

```bash
mcaedit session create --world /path/to/world --id demo --label agent-a
mcaedit --session demo inspect select --from 0,64,0 --to 7,66,7
mcaedit --session demo edit set-block --x 1 --y 64 --z 2 --block minecraft:stone
mcaedit --session demo history undo --n 1
mcaedit --session demo template save --name hut --from 0,64,0 --to 5,67,5
mcaedit --session demo commit
```

## Multi-session

```bash
mcaedit session list
mcaedit --session bob session sync          # pull others' commits
mcaedit --session bob session sync --force  # overwrite dirty work copy
```

## Template

```bash
mcaedit template list
mcaedit --session demo template paste --name hut --at 32,64,32
```

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/CntierTeam/MCAEdit/main/scripts/install.sh | bash
```
