# WorldEdit / Pumpkin → MCAEdit

方块选区：`--from x,y,z --to x,y,z`（无 wand）。
Pumpkin：`--from cx,cz --to cx,cz`（chunk）。

| 概念 | MCAEdit |
|------|---------|
| `//set` / `//fill` | `edit fill --block` |
| `//replace` | `edit replace --match --with` |
| `//walls` | `edit walls` |
| `//faces` | `edit outline` |
| `//hollow` | `edit hollow` |
| `//overlay` | `edit overlay` |
| `//sphere` / `//hsphere` | `edit sphere [--hollow]` |
| `//cyl` / `//hcyl` | `edit cyl [--hollow]` |
| `//stack` | `edit stack --n --dx/--dy/--dz` |
| `//move` | `edit move --dx/--dy/--dz` |
| `//copy`/`//cut`/`//paste` | `edit copy`/`cut`/`paste --at` |
| `//rotate`/`//flip` | `edit rotate --yaw` / `flip --axis`（剪贴板） |
| `//undo`/`//redo` | `history undo`/`redo` |
| schematic | `template save`/`paste` |
| 地形 gen | `edit gen --seed --dim --from/--to` |
| 重光照 | `edit fix-light` |
| tick | `edit tick-participate` |

未实现：brush、复杂 mask、% pattern、smooth、`.schem`。
