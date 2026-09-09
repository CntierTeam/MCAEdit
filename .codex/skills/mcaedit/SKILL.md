---
name: mcaedit
description: >-
  Operate offline Minecraft Anvil (.mca) editor CLI `mcaedit` by running it for
  the user: session create/sync, inspect, WorldEdit-style region ops
  (fill/replace/walls/outline/hollow/overlay/sphere/cyl/stack/move), clipboard
  (copy/cut/paste/rotate/flip), templates, history undo/redo, Pumpkin gen /
  fix-light / tick-participate, and commit. Prefer shell execution over pasting
  recipes. Trigger on: MCAEdit, mcaedit, MCA editor, Anvil region edit,
  WorldEdit offline, chunk palette, inspect select ASCII.
license: GPL-3.0
metadata:
  short-description: 代跑 mcaedit（session/编辑/commit）
---

# MCAEdit

产品：**`mcaedit`** — 离线 Minecraft Anvil（`.mca`）编辑 CLI（**GPL-3.0**，链接 [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin)）。

你是 **操作员**：用户要开 session、inspect、填方/替换/几何编辑、剪贴板、模板、undo、地形 gen、fix-light、tick、commit → **自己在 shell 执行 `mcaedit`**，不要只拼命令或讲 crate 布局。

本 skill 是 **execute-first**：代跑产品，不是开发 MCAEdit crates。WorldEdit 对照见 [references/worldedit-map.md](references/worldedit-map.md)。

Repo: https://github.com/CntierTeam/MCAEdit

流程：`session` → `inspect` / `edit` → `history` → `commit`。

## Agent 硬规则

1. **执行优先**：能跑就跑。二进制：`mcaedit` 或 `~/.local/bin/mcaedit`；没有就先装。
2. **禁止**用「组装指令 / SAMPLE / YOUR_CLI / crates 开发手册」代替执行。短句说明 → 立刻跑 → 根据输出继续。
3. **永远用 session**：`--session` 或 `MCAEDIT_SESSION`。编辑落在工作副本；**`commit`** 才写回源世界。
4. 命令名永远 **`mcaedit`**，禁止 `SAMPLE` / `YOUR_CLI`。缺世界路径、session id、坐标、方块 id 时只问缺的那一项，问完继续跑。
5. `inspect select` 给 LLM 看时优先小盒（≤16³）。多 session：一 agent 一 label；他人 commit 后对本 session `session sync`。
6. `minecraft:void_air` 在 `set-section` = 保留原方块；`replace --match air` = air-like。
7. `edit gen` / `fix-light` / `tick-participate` 用 **chunk 坐标**（`--from x,z --to x,z`），不是方块 AABB。方块编辑用 `--from x,y,z --to x,y,z`。
8. 破坏性 commit / discard 意图不清时先确认；可用 `commit --dry-run`。

## 标准代跑流

```bash
command -v mcaedit || ~/.local/bin/mcaedit --help

mcaedit session create --world /path/to/world --id demo --label agent
export MCAEDIT_SESSION=demo   # 或每条命令加 --session demo

mcaedit inspect summary --cx 0 --cz 0
mcaedit inspect select --from 0,64,0 --to 7,66,7

mcaedit edit fill --from 0,64,0 --to 7,66,7 --block minecraft:stone
mcaedit history list
mcaedit commit            # 或先 commit --dry-run
```

## 意图 → 怎么跑

| 用户意图 | 执行 |
|----------|------|
| 开 / 列 / 同步 session | `session create\|list\|status\|sync\|discard` |
| 看区块 / 方块 / ASCII 选区 | `inspect summary\|get\|slice\|select\|palette\|entities` |
| 填方 / 替换 / 墙 / 外框 / 挖空 / 覆盖 | `edit fill\|replace\|walls\|outline\|hollow\|overlay` |
| 球 / 柱 / 堆叠 / 移动 | `edit sphere\|cyl\|stack\|move` |
| 剪贴板 | `edit copy\|cut\|paste\|rotate\|flip` |
| 单方块 / section / 实体 | `edit set-block`；`set-section`；`edit entity …` |
| 地形 gen / 重光照 / tick | `edit gen`；`fix-light`；`tick-participate`（chunk 坐标） |
| 模板 | `template save\|list\|show\|paste\|rm` |
| 撤销 / 重做 | `history undo\|redo\|revert\|list` |
| 写回世界 | `commit`（可先 `--dry-run`） |

## Session / multi-agent

```bash
mcaedit session create --world /path/to/world --id alice --label agent-a
mcaedit session create --world /path/to/world --id bob --label agent-b
mcaedit session list
mcaedit --session alice edit set-block --x 0 --y 64 --z 0 --block minecraft:stone
mcaedit --session alice commit
mcaedit --session bob session sync
```

工作副本：`./.mcaedit/<id>/`（含 `clipboard.json`）。模板：`./.mcaedit/templates/`。Env：`MCAEDIT_SESSION=<id>`。

## Inspect

```bash
mcaedit inspect summary --cx 0 --cz 0
mcaedit inspect get --x 1 --y 64 --z 2
mcaedit inspect slice --y 64 --from 0,0 --to 8,8
mcaedit inspect select --from 0,64,0 --to 7,66,7
mcaedit inspect palette --cx 0 --cz 0 --sy 4
mcaedit inspect entities --near 0,64,0 --r 32
```

## WorldEdit-style（方块 AABB）

选区一律 `--from x,y,z --to x,y,z`（无 wand）。对照表：[references/worldedit-map.md](references/worldedit-map.md)。

```bash
mcaedit --session demo edit fill --from 0,64,0 --to 7,66,7 --block minecraft:stone
mcaedit --session demo edit replace --from 0,64,0 --to 7,66,7 \
  --match air --with minecraft:glass
mcaedit --session demo edit walls --from 0,64,0 --to 7,66,7 --block minecraft:oak_planks
mcaedit --session demo edit outline --from 0,64,0 --to 7,66,7 --block minecraft:stone
mcaedit --session demo edit hollow --from 0,64,0 --to 7,66,7
mcaedit --session demo edit overlay --from 0,64,0 --to 7,70,7 --block minecraft:snow
mcaedit --session demo edit sphere --at 0,70,0 --radius 5 --block minecraft:glass
mcaedit --session demo edit sphere --at 0,70,0 --radius 5 --block minecraft:glass --hollow
mcaedit --session demo edit cyl --at 0,0 --y 64 --radius 4 --height 8 --block minecraft:stone
mcaedit --session demo edit stack --from 0,64,0 --to 2,64,2 --n 3 --dx 4 --dy 0 --dz 0
mcaedit --session demo edit move --from 0,64,0 --to 2,64,2 --dx 8 --dy 0 --dz 0
```

Clipboard：

```bash
mcaedit --session demo edit copy --from 0,64,0 --to 3,65,2
mcaedit --session demo edit rotate --yaw 90
mcaedit --session demo edit flip --axis x
mcaedit --session demo edit paste --at 32,64,32
mcaedit --session demo edit cut --from 0,64,0 --to 3,65,2
```

## Pumpkin（chunk 坐标）

```bash
mcaedit --session demo edit gen --seed 42 --dim overworld --from 0,0 --to 3,3
mcaedit --session demo edit fix-light --from 0,0 --to 3,3 --seed 42 --dim overworld
mcaedit --session demo edit tick-participate --from 0,0 --to 3,3 --rounds 20 --speed 3
```

`--dim`：`overworld` | `nether` | `end`。完整作物随机刻行为在 Pumpkin 服务端；离线 `tick-participate` 重建 mask、采样候选、步进 scheduled tick。

## Templates / history / commit

```bash
mcaedit --session demo template save --name hut --from 0,64,0 --to 5,67,5
mcaedit template list
mcaedit --session demo template paste --name hut --at 32,64,32

mcaedit history list
mcaedit history undo --n 1
mcaedit history redo --n 1
mcaedit history revert --to 0
mcaedit commit
mcaedit commit --dry-run
```

部分命令支持 `--json`。

## Install（仅当本机没有 mcaedit）

```bash
curl -fsSL https://raw.githubusercontent.com/CntierTeam/MCAEdit/main/scripts/install.sh | bash
command -v mcaedit && mcaedit --help
# 源码安装（需同级 ../Pumpkin）：./scripts/install.sh --from-source --symlink-skill --force
```

## Out of scope（产品能力边界）

- Brush / 复杂 mask / % patterns / smooth / biomes / `.schem`
- 完整服务端 random-tick 方块行为
- Linear region 格式；heightmaps/lighting 除 `fix-light` 外不自动重算

https://github.com/CntierTeam/MCAEdit
