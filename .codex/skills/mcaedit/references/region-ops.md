# 区域操作 → MCAEdit

方块选区：`--from x,y,z --to x,y,z`（无 wand）。
Brush：`--at` + `--radius`（柱再加 `--height`）。
地形 / 光照 / tick：`--from cx,cz --to cx,cz`（chunk）。

## 负坐标（P1）

| 形式 | 示例 | 何时用 |
|------|------|--------|
| `=`（推荐） | `--from=-8,60,-8 --to=7,70,7` | 永远安全；给脚本/示例优先写这个 |
| 空格（≥0.9.0） | `--from -8,60,-8 --to 7,70,7` | `allow_hyphen_values`；旧二进制仍可能炸 |

同理：`--at` / `--near` / `--camera` / `--look`。看到 `unexpected argument '-8,...'` → 升级到 0.9.0+ 或改用 `=`。

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
| **柱网** | `edit grid`（别名 `colonnade`）`--spacing-x/--spacing-z` |
| **瓦垄** | `edit roof-rows --axis --period [--stairs --stairs-facing]` |
| **定向楼梯** | `edit stairs --facing [--half] [--shape]` |
| 堆叠 | `edit stack --n --dx/--dy/--dz` |
| 移动 | `edit move --dx/--dy/--dz` |
| 复制/剪切/粘贴 | `edit copy`/`cut`/`paste --at` |
| 旋转/翻转 | `edit rotate --yaw` / `flip --axis`（剪贴板） |
| 撤销/重做 | `history undo`/`redo` |
| 模板 | `template save`/`paste`/`export-schem`/`import-schem` |
| `.schem` | `schem export`/`import`/`info`（Sponge v2 写；v2/v3 读） |
| **结构 `.nbt`** | `structure list\|info\|place\|export\|import\|clear-refs`（`--rotation` / `--mirror`） |
| **新建世界 / level.dat** | `world create`；`level info\|write\|patch`；`session create --bootstrap`（已有 level.dat 复用） |
| 地形 gen | `edit gen --seed --dim --from/--to` |
| 重光照 | `edit fix-light`（≥0.8.2：**只**改 light / isLightOn；不 gen、不改方块；**0.10 截图前建议跑**） |
| **离线 tick** | `edit tick`（别名 `tick-participate`） |
| 小选区 ASCII | `inspect select`（约 ≤48³；xz 宽有上限） |
| **大体积验收** | `inspect summary-box`（方块计数，无 ASCII） |
| 离线截图 | `view screenshot`（0.10：**models+textures** + BlockLight/SkyLight；`--minecraft\|--assets-jar` / `--max-cells` / `--no-textures`） |
| **实时预览** | `view preview` / `preview`（同管线；`--watch`；需显示器） |
| Linear region | session 打开 `.linear`；commit 按源格式写回；`world create --region-format linear` |
| DataVersion | `--mc 26.2`（默认 4903）及 1.18.2–1.21.x 别名；或裸 `--mc 3465` |
| 多 session | `session lease` / `lease-list` / `lease-clear`；`commit` 可能 `warn=` 重叠 lease / 较新 mtime |

### 截图 / 预览速记（v0.10.0）

- 几何：blockstates + models（楼梯/台阶/栅栏/门/交叉植物等），非 cubes-only；缺 jar → 调色板立方体。
- 光照：`BlockLight`/`SkyLight` × lightmap × face shade + 简化 AO → 先 `fix-light`。
- 默认 max-cells **2e6**；`--max-cells` / `MCAEDIT_VIEW_MAX_CELLS`（**不是** 48³ 硬限）。
- 日志：`textures=models+textures jar path=...` vs `textures=palette reason=...`。
- 仍缺：CTM、流体曲面、实体方块特殊渲染、动画仅首帧、完整邻域 AO。
