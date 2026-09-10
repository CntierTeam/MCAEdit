---
name: mcaedit
description: >-
  Operate offline Minecraft Anvil (.mca) / Linear (.linear) editor CLI `mcaedit`
  by running it for the user across all actions: session create/sync/discard,
  inspect, region ops (fill/replace/walls/outline/hollow/overlay/sphere/cyl/stack/move),
  brush (sphere/cyl/clipboard/biome), mask/% pattern, smooth / smooth3d, biome paint,
  clipboard, templates, schem import/export, structure .nbt, level.dat / world create,
  history undo/redo, view screenshot, view preview (live window), terrain gen / fix-light /
  tick, and commit. Prefer shell execution over pasting recipes. Trigger on: MCAEdit, mcaedit,
  MCA, Anvil, Linear region, offline region edit, level.dat, structure, 结构, view screenshot,
  view preview, 截图, 预览, brush, schem, biome, smooth, smooth3d, 地形生成, fix-light, tick,
  chunk palette, inspect select, summary-box, 负坐标, grid, colonnade, roof-rows.
license: GPL-3.0
metadata:
  short-description: 代跑 mcaedit（session/edit/view/commit）
---

# MCAEdit

产品：**`mcaedit`** — 离线 Minecraft Anvil（`.mca`）/ Linear（`.linear`）编辑 CLI（**GPL-3.0**）。
**当前版本：0.9.0**（`mcaedit --version`；与 workspace `Cargo.toml` 对齐）。

你是 **操作员**：用户要开 session、inspect、填方/替换/几何、brush、mask/% pattern、smooth/smooth3d、biome、剪贴板、模板、`.schem`、**结构 `.nbt`**、**level.dat / world create**、undo、地形 gen、fix-light、**tick**、view 截图 / **实时 preview**、commit → **自己在 shell 执行 `mcaedit`**，不要只拼命令给用户。

## Agent 硬规则

1. **执行优先（全部功能）**：能跑就跑。二进制：`mcaedit` 或 `~/.local/bin/mcaedit`；没有就先装：
   `curl -fsSL https://raw.githubusercontent.com/CntierTeam/MCAEdit/main/scripts/install.sh | bash`
   或仓库内：`./scripts/install.sh --from-source --symlink-skill --force`（会先 `scripts/ensure-vendor.sh`）。
2. **禁止**用「组装指令 / 操作手册 / SAMPLE / YOUR_CLI / crates 开发讲义 / 我只能帮你校验」代替执行。短句说明 → 立刻跑 → 根据输出继续。
3. 用户问「能不能填方 / brush / smooth / biome / schem / structure / 建世界 / 生成地形 / 修光 / tick / 截图 / 预览 / commit」→ **先答能**，再 **马上执行**。缺世界路径、session id、坐标、方块 id、seed 时只问缺的那一项，问完继续跑。
4. 命令名永远 **`mcaedit`**，禁止 `SAMPLE` / `YOUR_CLI`。
5. **所有**动作都要会代跑：`session` `inspect` `edit`（含 brush/mask/%pattern/smooth/smooth3d/biome/gen/fix-light/tick/grid/roof-rows/stairs）`schem` `structure` `world` `level` `template` `view`/`preview` `history` `commit`。
6. **永远用 session**：`--session <id>` 或 `export MCAEDIT_SESSION=<id>`。编辑只改工作副本；**`commit`** 才写回源世界。可先 `commit --dry-run`。
7. 方块 AABB 用 `--from x,y,z --to x,y,z`；brush 用 `--at` + `--radius`；`gen` / `fix-light` / `tick` 用 **chunk** `--from x,z --to x,z`。
   **负坐标见下方 P1 专节**（`=` 最稳；≥0.9.0 空格形式也可）。
8. `inspect select` 优先 ≤16³（硬上限约 48³；更大用 `inspect summary-box` / `view screenshot`）。多 agent：一人一 session label；他人 commit 后对本 session `session sync`。
9. `minecraft:void_air`（set-section）= 保留；`replace --match air` / `--mask air` = air-like。`--pattern '50%stone,50%dirt'` 支持加权。破坏性 `discard` / 大范围 gen 意图不清时先确认一句。

## 负坐标（P1）— 必读

clap 会把以 `-` 开头的 token 当成 flag。坐标参数已全部加了 `allow_hyphen_values`，**自 v0.9.0 起两种写法都合法**；旧文档/习惯常说「空格形式会炸」，那是 ≤0.8.x 的坑。

| 形式 | 示例 | 说明 |
|------|------|------|
| **`=` 形式（永远最稳）** | `--from=-8,60,-8` | 推荐默认；不确定时一律用这个 |
| **空格形式（≥0.9.0 OK）** | `--from -8,60,-8` | 现已可；老脚本/老记忆别再回避 |

同样适用于：`--from` / `--to` / `--at` / `--near` / `--camera` / `--look`（以及其它 `x,y,z` / `x,z` 坐标 long 参数）。

```bash
# ✅ 推荐（=）：不会被 clap 吃成 flag
mcaedit edit fill --from=-8,60,-8 --to=7,70,7 --block minecraft:stone
mcaedit edit brush sphere --at=-4,72,-4 --radius 5 --block minecraft:glass --mask air
mcaedit inspect entities --near=-16,64,-16 --r 32
mcaedit view screenshot --from=-10,55,-55 --to=45,100,12 --out /tmp/shot.png \
  --camera=-28,105,-78 --look=16,78,-22

# ✅ 空格形式（v0.9.0+，allow_hyphen_values）
mcaedit edit fill --from -8,60,-8 --to 7,70,7 --block minecraft:stone
mcaedit edit brush sphere --at -4,72,-4 --radius 5 --block minecraft:glass

# ❌ 老习惯误区：以为空格必炸 → 改用 = 或升级到 ≥0.9.0
# 若仍看到 unexpected argument '-8,...' → 二进制太旧，先升级；或立刻改用 --from=-8,...
```

**实务**：给用户/脚本示例优先写 `=`；自己代跑两种都行。chunk 坐标同理：`--from=-2,-2 --to=1,1`。

## Agent 易踩坑（v0.8.2–0.9.0 实务）

### `fix-light` 只改光（≥0.8.2）

`edit fix-light` **只重算 sky/block light**（并维护 `isLightOn`），**不**改方块、调色板、实体。v0.8.2 已修「修光把编辑方块冲掉」的 bug。

- **不要**因为怕抹掉建筑而跳过 fix-light（在 ≥0.8.2 / 0.9.0 上该跑就跑）。
- `--seed` / `--dim` **仅定维度高度范围**，**不会**触发 `gen`。

```bash
mcaedit edit fix-light --from 0,0 --to 3,3 --seed 42 --dim overworld
mcaedit edit fix-light --from=-2,-2 --to=1,1 --dim overworld
```

### 截图贴图 / 大体积

- `--minecraft <jar|versions/dir>` 或 `MCAEDIT_MINECRAFT_JAR` / `MCAEDIT_ASSETS_JAR`；可省略，自动探测 26.2。
- 日志：`textures=jar path=...`（贴图成功）vs `textures=palette reason=no-textures|jar-not-found|...`（回退实色，不崩溃）。
- **无 48³ 硬上限**（那是 `inspect select` / preview 自动裁的旧习惯）。大 AABB 用 `--max-cells` 或 `MCAEDIT_VIEW_MAX_CELLS`（默认 **2000000**）。超大体积 mesh 会慢，可先缩小选区或降分辨率。

```bash
mcaedit view screenshot --from=-10,55,-55 --to=45,100,12 --out /tmp/taihe.png \
  --width 1600 --height 900 --camera=-28,105,-78 --look=16,78,-22 \
  --minecraft /other/Minecraft/.minecraft/versions/26.2/26.2.jar \
  --max-cells 4000000
# 或：export MCAEDIT_VIEW_MAX_CELLS=4000000
```

### `world create` vs `session create --bootstrap`

| 场景 | 命令 |
|------|------|
| **新建**空世界骨架 | `world create --path …`（写 level.dat + region/） |
| 目录已有 / 可能已有 **level.dat** | `session create --world … --bootstrap`：**缺啥补啥**；已有 level.dat **复用不覆盖** |

```bash
mcaedit world create --path /tmp/newworld --name Demo --seed 42 --mc 26.2 --generator flat
mcaedit session create --world /tmp/newworld --id demo --label agent --bootstrap --mc 26.2
# 已有世界：直接 session create [--bootstrap]；别用 world create 覆盖
```

### 多 session：lease / commit 冲突

```bash
mcaedit session create --world /path/to/world --id alice --label agent-a
mcaedit session create --world /path/to/world --id bob --label agent-b
mcaedit --session alice session lease --from=-32,60,-32 --to=32,90,32
# lease 重叠会打印 warn=lease conflict: ...
mcaedit --session alice commit   # 可能 warn=region conflict hint: ... mtime newer; 或 lease overlap
mcaedit --session bob session sync
mcaedit session lease-list
mcaedit --session alice session lease-clear
```

软租赁（`.mcaedit/leases/`）不硬锁；看到 `warn=` 先协调 / `sync`，再 commit。

### 建造助手

```bash
# 柱网（别名 colonnade）
mcaedit edit grid --from=0,64,0 --to=31,72,31 --spacing-x 4 --spacing-z 4 --block minecraft:oak_log
mcaedit edit colonnade --from=0,64,0 --to=31,72,31 --spacing-x 4 --spacing-z 4 --block minecraft:oak_log

# 瓦垄：奇数行可选楼梯朝向
mcaedit edit roof-rows --from=0,80,0 --to=31,80,31 --axis z --period 2 \
  --block minecraft:brick_slab --stairs minecraft:brick_stairs --stairs-facing north

# 定向楼梯填充
mcaedit edit stairs --from=0,64,0 --to=7,64,0 --block minecraft:oak_stairs --facing east --half bottom
```

### 大体积验收：`inspect summary-box`

`inspect select` 有体积/宽度上限且出 ASCII；大建筑验收用 **summary-box**（方块计数，无 ASCII）。

```bash
mcaedit inspect summary-box --from=-10,55,-55 --to=45,100,12
mcaedit inspect select --from=0,64,0 --to=7,66,7   # 小选区可视化
```

## 标准代跑流

```bash
command -v mcaedit || ~/.local/bin/mcaedit --help
mcaedit --version   # 期望 0.9.0+

# 空目录建世界骨架（level.dat + region/）；默认 MC 26.2 DataVersion=4903
mcaedit world create --path /tmp/newworld --name Demo --seed 42 --mc 26.2 --generator flat
# 或：老 Anvil 格式
mcaedit world create --path /tmp/oldworld --mc 1.18.2 --seed 1 --generator noise
mcaedit level info --world /tmp/newworld
mcaedit level patch --world /tmp/newworld --name Renamed --seed 99 --touch

# world create = 新建世界；session --bootstrap = 空目录补齐可用骨架（已有 level.dat 则复用，不覆盖）
mcaedit session create --world /tmp/newworld --id demo --label agent --bootstrap --mc 26.2
export MCAEDIT_SESSION=demo

mcaedit inspect select --from 0,64,0 --to 7,66,7
mcaedit edit fill --from 0,64,0 --to 7,66,7 --pattern '50%stone,50%dirt'
mcaedit edit brush sphere --at 0,70,0 --radius 5 --block minecraft:glass --mask air
mcaedit edit brush biome --at 0,70,0 --radius 8 --biome minecraft:desert
mcaedit edit smooth3d --from 0,60,0 --to 15,80,15 --iterations 2 --kernel 1
mcaedit edit tick --from 0,0 --to 0,0 --rounds 20 --speed 3
mcaedit edit fix-light --from 0,0 --to 0,0 --dim overworld

# 原版结构 .nbt（≠ .schem）
mcaedit structure export --from 0,64,0 --to 7,70,7 --out /tmp/hut.nbt
mcaedit structure place --file /tmp/hut.nbt --at 32,64,32 --rotation 90 --mirror x
mcaedit structure list --world /tmp/newworld
mcaedit inspect structures --cx 0 --cz 0
mcaedit structure clear-refs --from 0,0,0 --to 63,0,63

mcaedit view screenshot --from 0,64,0 --to 7,66,7 --out /tmp/shot.png --width 640 --height 360
# 大体积 + 负坐标验收（推荐 = 形式）
mcaedit view screenshot --from=-10,55,-55 --to=45,100,12 --out /tmp/taihe.png \
  --width 1600 --height 900 --camera=-28,105,-78 --look=16,78,-22 \
  --minecraft /other/Minecraft/.minecraft/versions/26.2/26.2.jar
# 使用本机 Minecraft 26.2 client jar 方块贴图（可省略，自动探测）
mcaedit view screenshot --from 0,64,0 --to 7,66,7 --out /tmp/shot.png \
  --minecraft ~/.minecraft/versions/26.2/26.2.jar
# 另开终端：实时建造预览窗口（需 DISPLAY/Wayland）
mcaedit view preview --from 0,64,0 --to 15,80,15 --watch 400
# 或：mcaedit preview --watch 500 --assets-jar /path/to/26.2.jar
mcaedit history list
mcaedit commit --dry-run
mcaedit commit
```

## 意图 → 怎么跑

| 用户意图 | 执行 |
|----------|------|
| **新建世界 / level.dat** | `world create` = 新世界；`session create --bootstrap` = 缺啥补啥（已有 level.dat 则复用）；`level info\|write\|patch` |
| 开 / 列 / 状态 / 同步 / 丢弃 session | `session create\|list\|status\|sync [--force]\|discard` |
| 软租赁（多 agent） | `session lease\|lease-list\|lease-clear` |
| 看区块 / 方块 / ASCII 选区 / 实体 / 结构引用 | `inspect summary\|get\|slice\|select\|palette\|entities\|structures` |
| **大体积方块统计** | `inspect summary-box --from/--to`（无 ASCII；验收用） |
| 填方 / 替换 / 墙 / 外框 / 挖空 / 覆盖 | `edit fill\|replace\|walls\|outline\|hollow\|overlay`（fill/replace 支持 `--pattern` / `--mask`） |
| 球 / 柱 / 堆叠 / 移动 | `edit sphere\|cyl\|stack\|move` |
| **Brush** | `edit brush sphere\|cyl\|clipboard\|biome\|biome-cyl --at …` |
| **Smooth** | `edit smooth --from/--to [--iterations] [--kernel]`（heightmap） |
| **Smooth3d** | `edit smooth3d --from/--to [--iterations] [--kernel] [--solid]`（体素多数表决） |
| **Biome paint** | `edit biome --from/--to --biome minecraft:plains` |
| **柱网 / 瓦垄 / 楼梯** | `edit grid`（别名 `colonnade`）/ `roof-rows` / `stairs --facing` |
| 剪贴板 | `edit copy\|cut\|paste --at\|rotate --yaw\|flip --axis` |
| 单方块 / section / 实体 | `edit set-block`；`set-section`；`edit entity spawn\|rm\|set` |
| **地形生成** | `edit gen --seed N --dim overworld\|nether\|end --from cx,cz --to cx,cz` |
| **修光照** | `edit fix-light --from cx,cz --to cx,cz [--seed] [--dim]`（只重算 light；≥0.8.2 不毁方块） |
| **离线 tick** | `edit tick --from cx,cz --to cx,cz --rounds N --speed N`（别名 `tick-participate`） |
| **离线截图** | `view screenshot --from/--to [--out] [--width] [--height] [--camera] [--look] [--minecraft\|--assets-jar] [--max-cells] [--no-textures]` |
| **实时预览** | `view preview` / `preview`（`--from/--to` 可选，`--watch` 轮询 ms；`--minecraft` / `--assets-jar`；需显示器） |
| 模板 | `template save\|list\|show\|paste\|rm\|export-schem\|import-schem` |
| **`.schem`** | `schem export\|import\|info`（Sponge v2 写出；读 v2/v3） |
| **结构 `.nbt`** | `structure list\|info\|place\|export\|import\|clear-refs`（原版 structure；place 可 `--rotation` / `--mirror`） |
| 撤销 / 重做 / 回退 | `history undo\|redo\|revert\|list` |
| 写回世界 | `commit`（可先 `--dry-run`；注意 `warn=` 冲突提示） |

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

## Biome brush / smooth3d / tick / preview

```bash
# Biome brush：世界坐标球形/柱形，写入 4×4×4 biome cell；可选 mask（按 cell 原点方块）
mcaedit edit brush biome --at 0,70,0 --radius 8 --biome minecraft:desert
mcaedit edit brush biome-cyl --at 0,0 --y 64 --radius 6 --height 16 --biome plains --mask air

# smooth3d：AABB 内 Chebyshev 邻域多数表决；--solid 先投 air/solid
mcaedit edit smooth3d --from 0,60,0 --to 31,80,31 --iterations 2 --kernel 1
mcaedit edit smooth3d --from 0,60,0 --to 31,80,31 --iterations 3 --kernel 1 --solid

# 离线 tick（一等公民）：步进 block_ticks/fluid_ticks + 近似 random-tick 生长（可 undo）
mcaedit edit tick --from 0,0 --to 3,3 --rounds 40 --speed 3

# 实时建造预览（另开终端；需 DISPLAY 或 Wayland；与 edit 并行看进度）
mcaedit view preview --from 0,64,0 --to 31,80,31 --watch 400
# 或：mcaedit preview --watch 500

# Minecraft 26.2 方块贴图（client jar）；缺 jar 时自动回退调色板实色，不崩溃
export MCAEDIT_MINECRAFT_JAR=~/.minecraft/versions/26.2/26.2.jar
# 或：export MCAEDIT_ASSETS_JAR=...
mcaedit view screenshot --from 0,64,0 --to 15,80,15 --out /tmp/shot.png --minecraft ~/.minecraft/versions/26.2
mcaedit view preview --assets-jar ~/.minecraft/versions/26.2/26.2.jar
# 强制纯色：--no-textures
```

自动探测路径（版本默认 **26.2**）：`/other/Minecraft/.minecraft/versions/26.2/26.2.jar`（buildTest 机）、`~/.minecraft/versions/26.2/26.2.jar`、Flatpak Mojang、Prism/PolyMC/MultiMC、`%APPDATA%/.minecraft/...`、HMCL 等。`--minecraft` 可指向 jar 或 `versions/26.2/` 目录。
环境变量：`MCAEDIT_MINECRAFT_JAR` / `MCAEDIT_ASSETS_JAR`；大截图体积：`MCAEDIT_VIEW_MAX_CELLS`（默认 2000000）。
日志：`textures=jar path=...` 或 `textures=palette reason=no-textures|jar-not-found|...`。

v1 限制：立方体贴图 + face-cull，**无完整 block model**（台阶/栏杆/十字植物等仍按整方块或回退实色）；动画贴图只用首帧；光照为原版六面明暗阶梯（顶>南北>东西>底）× 全日空 lightmap，screenshot/preview 共用，无邻域 AO。

## Linear region

Session 可打开仅含 `r.X.Z.linear`（v1/v2）的世界：工作副本自动转成 `.mca` 编辑；`commit` 若源为 Linear 则写回 `.linear`。
`world create --region-format linear` 只建空目录（与 anvil 相同树）；首写区域文件时仍走现有 Linear 路径。

## level.dat / DataVersion

默认新建目标 **26.2**（DataVersion **4903**）。`--mc` / `--data-version` 可选：

| alias | DataVersion | level.dat 布局 |
|-------|-------------|----------------|
| 26.2 | 4903 | modern（另写 `data/minecraft/world_gen_settings.dat`；Data 内仍嵌 WorldGenSettings） |
| 1.21.11 | 4671 | modern |
| 1.21.10 | 4556 | modern |
| 1.21.4 | 4189 | classic（WorldGenSettings 在 Data） |
| 1.21.1 | 3955 | classic |
| 1.21 | 3738 | classic |
| 1.20.4 | 3700 | classic |
| 1.20.1 / 1.20 | 3465 / 3463 | classic |
| 1.19.4 | 3337 | classic |
| 1.18.2 | 2975 | classic |

也可直接 `--mc 3465`（裸 DataVersion）。编辑已有世界时 **保留** 区块原 DataVersion；空区块新建用 session `meta.data_version`（来自 level.dat / bootstrap）。

## 多 session

见上文「lease / commit 冲突」。工作副本：`./.mcaedit/<id>/`（含 `clipboard.json`）。模板：`./.mcaedit/templates/`。租赁：`./.mcaedit/leases/`。

## 安装（仅当本机没有 mcaedit）

```bash
curl -fsSL https://raw.githubusercontent.com/CntierTeam/MCAEdit/main/scripts/install.sh | bash
# 开发机源码：
./scripts/install.sh --from-source --symlink-skill --force
```

`--symlink-skill` 会把 `~/.codex/skills/mcaedit` 链到仓库 `.codex/skills/mcaedit`（改 SKILL 后无需再拷）。

## Out of scope / 诚实限制

- 完整服务端 random-tick（光照/湿度/邻居更新/蜜蜂授粉等）；离线 tick 覆盖作物 age、甘蔗/仙人掌/竹子向上长、草/菌丝扩散、farmland 湿度递减，以及 scheduled tick 队列步进（到期条目移除，不执行完整方块行为）
- 生物群系分辨率低于 4×4×4（MCA section biomes 固有限制）
- Linear：支持读/写 v1 与 v2；工作副本仍以 Anvil 编辑
- view 贴图：依赖本机 Minecraft client jar；无 jar 时调色板实色；非完整 blockstate 模型
- view 光照：MC 六面 shade + lightmap 曲线；无 smooth AO / 方块光传播
- **structure place**：稠密体积上限 64³；多 palette 结构只用第一套；旋转/镜像改方块坐标，**不**旋转方块 state（如楼梯朝向）
- **structure clear-refs**：清 chunk `structures` starts/References，**不入** history undo
- **level.dat**：写入常用字段（LevelName/Seed/Spawn/GameType/WorldGenSettings/Version…）；不保证与所有第三方服务端 sidecar 全集一致；现代布局额外写 `world_gen_settings.dat`
- **负坐标**：≤0.8.x 空格形式可能被 clap 当 flag；≥0.9.0 已修，仍推荐 `=`

https://github.com/CntierTeam/MCAEdit
