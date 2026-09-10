---
name: mcaedit-style-TEMPLATE
description: >-
  TEMPLATE — copy to styles/<slug>/SKILL.md when learning a build style via
  /learn or 标注. Replace all placeholders. Triggers: /use style <slug>, style name.
disable-model-invocation: true
---

# Style: `<slug>`

> Learned from: `<path/to/source.schem or world AABB>`  
> DataVersion / mc: `<e.g. 4903 / 26.2>`  
> Size: `<W×H×L>`

## When to use

User asks to build in this style, or says `/use style <slug>` / `按 <name> 风格`.

## Material palette

| Role | Blocks |
|------|--------|
| Structure / pillars | `minecraft:…` |
| Fill / walls | `minecraft:…` |
| Roof / cornice | `minecraft:…` (+ stairs/slabs) |
| Accent | `minecraft:…` |
| Floor | `minecraft:…` |

## Rhythm / proportions

- Footprint module: `<e.g. 4×4 bay>`
- Pillar spacing: `--spacing-x N --spacing-z N`
- Wall height (above floor): `<N>`
- Roof: `roof-rows` axis=`x|z` period=`N` stairs-facing=`north|…`

## Construction recipes (mcaedit ops)

Prefer rebuild with ops; use `schem import` only for exact paste.

```bash
export MCAEDIT_SESSION=<id>
# 1) footprint / floor
mcaedit edit fill --from=0,64,0 --to=15,64,15 --block minecraft:PLACEHOLDER

# 2) walls / shell
mcaedit edit walls --from=0,65,0 --to=15,72,15 --block minecraft:PLACEHOLDER

# 3) colonnade / grid
mcaedit edit colonnade --from=0,65,0 --to=15,72,15 --spacing-x 4 --spacing-z 4 \
  --block minecraft:PLACEHOLDER

# 4) roof
mcaedit edit roof-rows --from=0,73,0 --to=15,73,15 --axis z --period 2 \
  --block minecraft:PLACEHOLDER_SLAB --stairs minecraft:PLACEHOLDER_STAIRS \
  --stairs-facing north

# 5) oriented stairs (eaves / steps)
mcaedit edit stairs --from=0,64,0 --to=7,64,0 --block minecraft:PLACEHOLDER_STAIRS \
  --facing east --half bottom
```

Exact paste (optional):

```bash
mcaedit schem import --file /path/to/source.schem --at=64,64,64
```

## DO

- Keep material families and spacing from the table above
- Use `=` for negative coords (`--from=-8,64,-8`)
- `edit fix-light` before acceptance screenshots

## DON'T

- Swap in unrelated wood/stone families without user ask
- Ignore stair `facing` / slab `type` when rebuilding roofs
- Blow past the learned bay module without scaling intentionally

## Source analysis snapshot

Paste key lines from `schem info --style-hints` (or summary-box):

```
materials_top=…
families=…
stairs_facing=…
pillar_spacing_hint=…
```
