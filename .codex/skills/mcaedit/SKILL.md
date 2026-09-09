---
name: mcaedit
description: >-
  Operate offline Minecraft Anvil (.mca) / Linear (.linear) editor CLI `mcaedit`
  by running it for the user across all actions: session create/sync/discard,
  inspect, region ops (fill/replace/walls/outline/hollow/overlay/sphere/cyl/stack/move),
  brush (sphere/cyl/clipboard/biome), mask/% pattern, smooth / smooth3d, biome paint,
  clipboard, templates, schem import/export, history undo/redo, view screenshot,
  terrain gen / fix-light / tick, and commit. Prefer shell execution over pasting
  recipes. Trigger on: MCAEdit, mcaedit, MCA, Anvil, Linear region, offline region
  edit, view screenshot, 截图, brush, schem, biome, smooth, smooth3d, 地形生成,
  fix-light, tick, chunk palette, inspect select.
license: GPL-3.0
metadata:
  short-description: 代跑 mcaedit（session/edit/view/commit）
---

# MCAEdit

产品：**`mcaedit`** — 离线 Minecraft Anvil（`.mca`）/ Linear（`.linear`）编辑 CLI（**GPL-3.0**）。

你是 **操作员**：用户要开 session、inspect、填方/替换/几何、brush、mask/% pattern、smooth/smooth3d、biome、剪贴板、模板、`.schem`、undo、地形 gen、fix-light、**tick**、view 截图、commit → **自己在 shell 执行 `mcaedit`**，不要只拼命令给用户。

## Agent 硬规则

1. **执行优先（全部功能）**：能跑就跑。二进制：`mcaedit` 或 `~/.local/bin/mcaedit`；没有就先装：
   `curl -fsSL https://raw.githubusercontent.com/CntierTeam/MCAEdit/main/scripts/install.sh | bash`
   或仓库内：`./scripts/install.sh --from-source --symlink-skill --force`（会先 `scripts/ensure-vendor.sh`）。
2. **禁止**用「组装指令 / 操作手册 / SAMPLE / YOUR_CLI / crates 开发讲义 / 我只能帮你校验」代替执行。短句说明 → 立刻跑 → 根据输出继续。
3. 用户问「能不能填方 / brush / smooth / biome / schem / 生成地形 / 修光 / tick / 截图 / commit」→ **先答能**，再 **马上执行**。缺世界路径、session id、坐标、方块 id、seed 时只问缺的那一项，问完继续跑。
4. 命令名永远 **`mcaedit`**，禁止 `SAMPLE` / `YOUR_CLI`。
5. **永远用 session**：`--session <id>` 或 `export MCAEDIT_SESSION=<id>`。编辑只改工作副本；**`commit`** 才写回源世界。可先 `commit --dry-run`。
6. 方块 AABB 用 `--from x,y,z --to x,y,z`；brush 用 `--at` + `--radius`；`gen` / `fix-light` / `tick` 用 **chunk** `--from x,z --to x,z`。
7. `inspect select` 优先 ≤16³。多 agent：一人一 session label；他人 commit 后对本 session `session sync`。
8. `minecraft:void_air`（set-section）= 保留；`replace --match air` / `--mask air` = air-like。`--pattern '50%stone,50%dirt'` 支持加权。破坏性 `discard` / 大范围 gen 意图不清时先确认一句。

## 标准代跑流

```bash
command -v mcaedit || ~/.local/bin/mcaedit --help

mcaedit session create --world /path/to/world --id demo --label agent
export MCAEDIT_SESSION=demo

mcaedit inspect select --from 0,64,0 --to 7,66,7
mcaedit edit fill --from 0,64,0 --to 7,66,7 --pattern '50%stone,50%dirt'
mcaedit edit brush sphere --at 0,70,0 --radius 5 --block minecraft:glass --mask air
mcaedit edit brush biome --at 0,70,0 --radius 8 --biome minecraft:desert
mcaedit edit smooth3d --from 0,60,0 --to 15,80,15 --iterations 2 --kernel 1
mcaedit edit tick --from 0,0 --to 0,0 --rounds 20 --speed 3
mcaedit view screenshot --from 0,64,0 --to 7,66,7 --out /tmp/shot.png --width 640 --height 360
mcaedit history list
mcaedit commit --dry-run
mcaedit commit
```

## 意图 → 怎么跑

| 用户意图 | 执行 |
|----------|------|
| 开 / 列 / 状态 / 同步 / 丢弃 session | `session create\|list\|status\|sync [--force]\|discard` |
| 看区块 / 方块 / ASCII 选区 / 实体 | `inspect summary\|get\|slice\|select\|palette\|entities` |
| 填方 / 替换 / 墙 / 外框 / 挖空 / 覆盖 | `edit fill\|replace\|walls\|outline\|hollow\|overlay`（fill/replace 支持 `--pattern` / `--mask`） |
| 球 / 柱 / 堆叠 / 移动 | `edit sphere\|cyl\|stack\|move` |
| **Brush** | `edit brush sphere\|cyl\|clipboard\|biome\|biome-cyl --at …` |
| **Smooth** | `edit smooth --from/--to [--iterations] [--kernel]`（heightmap） |
| **Smooth3d** | `edit smooth3d --from/--to [--iterations] [--kernel] [--solid]`（体素多数表决） |
| **Biome paint** | `edit biome --from/--to --biome minecraft:plains` |
| 剪贴板 | `edit copy\|cut\|paste --at\|rotate --yaw\|flip --axis` |
| 单方块 / section / 实体 | `edit set-block`；`set-section`；`edit entity spawn\|rm\|set` |
| **地形生成** | `edit gen --seed N --dim overworld\|nether\|end --from cx,cz --to cx,cz` |
| **修光照** | `edit fix-light --from cx,cz --to cx,cz [--seed] [--dim]` |
| **离线 tick** | `edit tick --from cx,cz --to cx,cz --rounds N --speed N`（别名 `tick-participate`） |
| **离线截图** | `view screenshot --from x,y,z --to x,y,z [--out] [--width] [--height] [--camera] [--look]` |
| 模板 | `template save\|list\|show\|paste\|rm\|export-schem\|import-schem` |
| **`.schem`** | `schem export\|import\|info`（Sponge v2 写出；读 v2/v3） |
| 撤销 / 重做 / 回退 | `history undo\|redo\|revert\|list` |
| 写回世界 | `commit`（可先 `--dry-run`） |

区域对照：[references/region-ops.md](references/region-ops.md)。

## Pattern / Mask 速查

```bash
# % / 权重 pattern（fill、replace --with、brush）
mcaedit edit fill --from 0,64,0 --to 15,64,15 --pattern '50%stone,50%dirt'
mcaedit edit fill --from 0,64,0 --to 15,64,15 --pattern '3*stone,1*dirt'

# mask：air / !air / #solid / a|b / a&b / comma OR；可 --mask-exclude
mcaedit edit replace --from 0,64,0 --to 15,70,15 --mask 'stone,dirt' --with minecraft:glass
mcaedit edit brush sphere --at 0,70,0 --radius 4 --block minecraft:sand --mask '#solid' --mask-exclude bedrock
```

## Biome brush / smooth3d / tick

```bash
# Biome brush：世界坐标球形/柱形，写入 4×4×4 biome cell；可选 mask（按 cell 原点方块）
mcaedit edit brush biome --at 0,70,0 --radius 8 --biome minecraft:desert
mcaedit edit brush biome-cyl --at 0,0 --y 64 --radius 6 --height 16 --biome plains --mask air

# smooth3d：AABB 内 Chebyshev 邻域多数表决；--solid 先投 air/solid
mcaedit edit smooth3d --from 0,60,0 --to 31,80,31 --iterations 2 --kernel 1
mcaedit edit smooth3d --from 0,60,0 --to 31,80,31 --iterations 3 --kernel 1 --solid

# 离线 tick（一等公民）：步进 block_ticks/fluid_ticks + 近似 random-tick 生长（可 undo）
mcaedit edit tick --from 0,0 --to 3,3 --rounds 40 --speed 3
```

## Linear region

Session 可打开仅含 `r.X.Z.linear`（v1/v2）的世界：工作副本自动转成 `.mca` 编辑；`commit` 若源为 Linear 则写回 `.linear`。

## 多 session

```bash
mcaedit session create --world /path/to/world --id alice --label agent-a
mcaedit session create --world /path/to/world --id bob --label agent-b
mcaedit --session alice edit set-block --x 0 --y 64 --z 0 --block minecraft:stone
mcaedit --session alice commit
mcaedit --session bob session sync
```

工作副本：`./.mcaedit/<id>/`（含 `clipboard.json`）。模板：`./.mcaedit/templates/`。

## 安装（仅当本机没有 mcaedit）

```bash
curl -fsSL https://raw.githubusercontent.com/CntierTeam/MCAEdit/main/scripts/install.sh | bash
# 开发机源码：
./scripts/install.sh --from-source --symlink-skill --force
```

## Out of scope / 诚实限制

- 完整服务端 random-tick（光照/湿度/邻居更新/蜜蜂授粉等）；离线 tick 覆盖作物 age、甘蔗/仙人掌/竹子向上长、草/菌丝扩散、farmland 湿度递减，以及 scheduled tick 队列步进（到期条目移除，不执行完整方块行为）
- 生物群系分辨率低于 4×4×4（MCA section biomes 固有限制）
- Linear：支持读/写 v1 与 v2；工作副本仍以 Anvil 编辑

https://github.com/CntierTeam/MCAEdit
