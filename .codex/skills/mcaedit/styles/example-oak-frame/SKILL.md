---
name: mcaedit-style-oak-frame
description: >-
  Oak timber-frame hut style for MCAEdit: oak_log posts on 4-spacing, oak_planks
  walls, oak stairs+slab roof-rows. Use when user says /use style oak-frame,
  oak frame, 橡木框架, or asks to rebuild in this learned style.
disable-model-invocation: true
---

# Style: `oak-frame`

> Didactic example (not from a checked-in `.schem`). Use as a filled-template reference after `/learn`.

## When to use

`/use style oak-frame`、`按 oak-frame 风格`、`橡木框架小屋`.

## Material palette

| Role | Blocks |
|------|--------|
| Structure / pillars | `minecraft:oak_log[axis=y]` |
| Fill / walls | `minecraft:oak_planks` |
| Roof | `minecraft:oak_slab[type=bottom]` + `minecraft:oak_stairs` |
| Floor | `minecraft:oak_planks` |
| Accent | `minecraft:glass_pane` (optional windows) |

## Rhythm / proportions

- Footprint module: **4×4** bay
- Pillar spacing: `--spacing-x 4 --spacing-z 4`
- Wall height above floor: **7**
- Roof: `roof-rows` `--axis z --period 2 --stairs-facing north`

## Construction recipes

```bash
export MCAEDIT_SESSION=<id>
# floor 16×16
mcaedit edit fill --from=0,64,0 --to=15,64,15 --block minecraft:oak_planks
# walls
mcaedit edit walls --from=0,65,0 --to=15,71,15 --block minecraft:oak_planks
# posts
mcaedit edit colonnade --from=0,65,0 --to=15,71,15 --spacing-x 4 --spacing-z 4 \
  --block minecraft:oak_log
# roof ridge band
mcaedit edit roof-rows --from=0,72,0 --to=15,72,15 --axis z --period 2 \
  --block minecraft:oak_slab --stairs minecraft:oak_stairs --stairs-facing north
```

## DO

- Keep oak family; preserve 4-spacing posts
- `fix-light` before `view screenshot`

## DON'T

- Mix spruce/dark_oak without user intent
- Flatten roof to full blocks when stairs/slabs define the rhythm
