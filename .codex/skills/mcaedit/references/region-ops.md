# 区域操作 → MCAEdit

方块选区：`--from x,y,z --to x,y,z`（无 wand）。
地形 / 光照 / tick：`--from cx,cz --to cx,cz`（chunk）。

| 概念 | MCAEdit |
|------|---------|
| 填方 | `edit fill --block` |
| 替换 | `edit replace --match --with` |
| 墙 | `edit walls` |
| 外框 | `edit outline` |
| 挖空 | `edit hollow` |
| 覆盖 | `edit overlay` |
| 球 / 空心球 | `edit sphere [--hollow]` |
| 柱 / 空心柱 | `edit cyl [--hollow]` |
| 堆叠 | `edit stack --n --dx/--dy/--dz` |
| 移动 | `edit move --dx/--dy/--dz` |
| 复制/剪切/粘贴 | `edit copy`/`cut`/`paste --at` |
| 旋转/翻转 | `edit rotate --yaw` / `flip --axis`（剪贴板） |
| 撤销/重做 | `history undo`/`redo` |
| 模板 | `template save`/`paste` |
| 地形 gen | `edit gen --seed --dim --from/--to` |
| 重光照 | `edit fix-light` |
| tick | `edit tick-participate` |
| 离线截图 | `view screenshot --from/--to [--out] [--width] [--height]` |

未实现：brush、复杂 mask、% pattern、smooth、`.schem`。
