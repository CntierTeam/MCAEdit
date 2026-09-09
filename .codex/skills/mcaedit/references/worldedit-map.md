# WorldEdit / Pumpkin → MCAEdit map

Skill 为 execute-first 操作员代跑；本文件是对照备查，不是替代 shell 执行。

Selections for WE ops are always `--from x,y,z --to x,y,z` (no wand).
Pumpkin ops use **chunk** coords `--from x,z --to x,z`.

| WorldEdit / Pumpkin | MCAEdit |
|---------------------|---------|
| `//set` / `//fill` | `edit fill --block` |
| `//replace` | `edit replace --match --with` |
| `//walls` | `edit walls` |
| `//faces` / outline | `edit outline` |
| `//hollow` | `edit hollow` |
| `//overlay` | `edit overlay` |
| `//sphere` / `//hsphere` | `edit sphere [--hollow]` |
| `//cyl` / `//hcyl` | `edit cyl [--hollow]` |
| `//stack` | `edit stack --n --dx --dy --dz` |
| `//move` | `edit move --dx --dy --dz` |
| `//copy` | `edit copy` |
| `//cut` | `edit cut` |
| `//paste` | `edit paste --at` |
| `//rotate` | `edit rotate --yaw 90\|180\|270` (clipboard) |
| `//flip` | `edit flip --axis x\|y\|z` (clipboard) |
| `//undo` / `//redo` | `history undo` / `history redo` |
| schematics | `template save` / `template paste` |
| Pumpkin chunk gen | `edit gen --seed --dim --from/--to` |
| Pumpkin relight | `edit fix-light --from/--to` |
| Pumpkin tick participate | `edit tick-participate --rounds --speed` |

Not implemented: brushes, masks, patterns, smooth, naturalize, biomes paint, `.schem`.
Full random-tick block behaviour requires the Pumpkin server; offline tick-participate rebuilds masks and steps scheduled ticks.
