# 区域操作 → MCAEdit

方块选区：`--from x,y,z --to x,y,z`（无 wand）。
Brush：`--at` + `--radius`（柱再加 `--height`）。
地形 / 光照 / tick：`--from cx,cz --to cx,cz`（chunk）。

| 概念 | MCAEdit |
|------|---------|
| 填方 | `edit fill --block` / `--pattern`（可 `%` 权重） |
| 替换 | `edit replace --match/--mask --with/--pattern` |
| Mask | `--mask air\|!air\|#solid\|a\|b\|a&b`；`--mask-exclude` |
| 墙 | `edit walls` |
| 外框 | `edit outline` |
| 挖空 | `edit hollow` |
| 覆盖 | `edit overlay` |
| 球 / 空心球 | `edit sphere [--hollow]` |
| 柱 / 空心柱 | `edit cyl [--hollow]` |
| Brush 球/柱/剪贴板 | `edit brush sphere\|cyl\|clipboard` |
| **Biome brush** | `edit brush biome\|biome-cyl --biome …`（世界空间形状 → 4×4×4） |
| Smooth（heightmap） | `edit smooth [--iterations] [--kernel]` |
| **Smooth3d** | `edit smooth3d [--iterations] [--kernel] [--solid]` |
| Biome paint | `edit biome --biome`（section 4×4×4） |
| 堆叠 | `edit stack --n --dx/--dy/--dz` |
| 移动 | `edit move --dx/--dy/--dz` |
| 复制/剪切/粘贴 | `edit copy`/`cut`/`paste --at` |
| 旋转/翻转 | `edit rotate --yaw` / `flip --axis`（剪贴板） |
| 撤销/重做 | `history undo`/`redo` |
| 模板 | `template save`/`paste`/`export-schem`/`import-schem` |
| `.schem` | `schem export`/`import`/`info`（Sponge v2 写；v2/v3 读） |
| **结构 `.nbt`** | `structure list\|info\|place\|export\|import\|clear-refs`（`--rotation` / `--mirror`） |
| **新建世界 / level.dat** | `world create`；`level info\|write\|patch`；`session create --bootstrap` |
| 地形 gen | `edit gen --seed --dim --from/--to` |
| 重光照 | `edit fix-light`（只改 light / isLightOn；不 gen、不改方块） |
| **离线 tick** | `edit tick`（别名 `tick-participate`） |
| 离线截图 | `view screenshot --from/--to [--out] [--width] [--height] [--minecraft\|--assets-jar] [--no-textures]` |
| **实时预览** | `view preview` / `preview`（`--watch`；需显示器；可贴图 jar） |
| Linear region | session 打开 `.linear`；commit 按源格式写回；`world create --region-format linear` |
| DataVersion | `--mc 26.2`（默认 4903）及 1.18.2–1.21.x 别名；或裸 `--mc 3465` |
