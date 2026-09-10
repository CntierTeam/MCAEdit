# MCAEdit

Offline Minecraft Anvil (`.mca`) editor CLI for LLMs — **GPL-3.0**（含地形生成 / 光照）。

仓库：[CntierTeam/MCAEdit](https://github.com/CntierTeam/MCAEdit)

Flow: `world create` / `session create [--bootstrap]` → `inspect` / `edit` / `structure` / `view screenshot` / **`view preview`**（区域编辑 + **gen / fix-light / tick**）→ `history` → `commit`

本仓库同时提供 **Codex Skill**（`$mcaedit`）：**操作员代跑 / execute-first**——在 shell 直接跑 `mcaedit`（session / inspect / edit / commit），不是只拼命令。

## 许可

本项目使用 **GNU GPL v3**。分发二进制时须提供对应源码。

## 一键安装（从 GitHub Release）

```bash
curl -fsSL https://raw.githubusercontent.com/CntierTeam/MCAEdit/main/scripts/install.sh | bash
```

Windows（PowerShell）：

```powershell
iwr -useb https://raw.githubusercontent.com/CntierTeam/MCAEdit/main/scripts/install.ps1 -OutFile install.ps1
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Force
```

默认安装：

- 二进制 → `~/.local/bin/mcaedit`
- Codex skill → `~/.codex/skills/mcaedit`

```bash
curl -fsSL https://raw.githubusercontent.com/CntierTeam/MCAEdit/main/scripts/install.sh \
  | bash -s -- --version v0.11.1 --force
./scripts/install.sh --from-source --symlink-skill --force
./scripts/install.sh --uninstall
```

## 从源码构建

地形栈在仓库内 submodule：`vendor/pumpkin`（`scripts/ensure-vendor.sh` 会 init/浅克隆，修补嵌套 workspace 继承，并对 **pumpkin-data 做编译裁枝**）。

```bash
bash scripts/ensure-vendor.sh
rustup default stable
cargo build --release -p mcaedit-cli
./target/release/mcaedit --help
```

或：`./scripts/install.sh --from-source --force`（内部会先 ensure-vendor）。

### Pumpkin 裁枝（`minimal-pumpkin`，默认开）

MCAEdit **本来就不会链接** Pumpkin 服务端 / protocol / plugin / inventory 等 crate（workspace `exclude = ["vendor/pumpkin"]`，path dep 只有 `pumpkin-world` / `util` / `data` / `config`）。真正拖慢编译的是 `pumpkin-data` 的 **default features**（item / translation / advancement / registry / bedrock / … 等巨型生成代码）。

`ensure-vendor.sh` 默认（`MCAEDIT_PUMPKIN_MINIMAL=1`）会：

1. 把 `pumpkin-world` → `pumpkin-data` 改成 `default-features = false` + 仅 worldgen/光照所需 feature（block/chunk/dimension/fluid/tag/noise/structures/…）
2. 把未使用的 `loot_table` 模块改成 `cfg(feature = "loot_table")`（离线 gen/fix-light 不需要）
3. 去掉 `biome.rs` 里未使用的 `EntityType` import（否则会连锁拉起 entity→item 整图）
4. 把 end_city 鞘翅 item-frame 写成手写 NBT（避免启用整个 `item` feature）

| | 全量 `pumpkin-data` default | 裁枝后（本仓库默认） |
|--|--|--|
| 冷编译 `pumpkin-data`（本机实测） | ~542s / rlib ~175MB | ~79s / rlib ~79MB |
| 仍需完整 vendor **克隆** | 是（path 布局 + workspace） | 是（裁的是 **编译图**，不是 clone 体积） |
| `edit gen` / `fix-light` / `tick` | ✅ | ✅ |

```bash
# 默认裁枝
bash scripts/ensure-vendor.sh
cargo build -p mcaedit-cli

# 关闭裁枝（恢复 Pumpkin default features；更慢）
MCAEDIT_PUMPKIN_MINIMAL=0 bash scripts/ensure-vendor.sh
cargo clean -p pumpkin-data && cargo build -p mcaedit-cli
```

下一步（若仍嫌重）：把 `pumpkin-world` 再拆成 `lighting` / `generation` feature，或 sparse-checkout 丢掉 server crate 源码（只省磁盘，对链接图无增益）。

## 地形 / 光照 / tick

坐标为 **chunk**（`--from x,z --to x,z`）：

```bash
mcaedit --session demo edit gen --seed 42 --dim overworld --from 0,0 --to 3,3
mcaedit --session demo edit fix-light --from 0,0 --to 3,3 --seed 42 --dim overworld
mcaedit --session demo edit tick --from 0,0 --to 3,3 --rounds 40 --speed 3
```

- `gen`：Full 阶段写入 session 工作副本 `region/`
- `fix-light`：只重算天空/方块光（保留方块与调色板；`--seed`/`--dim` 仅定维度范围，不 gen）
- `tick`（别名 `tick-participate`）：步进 `block_ticks`/`fluid_ticks`，并对作物/甘蔗/草等做近似 random-tick 生长（生长变更可 undo）。**不是**完整服务端行为（无光照/湿度校验、无到期 tick 的完整方块逻辑）

`--dim`：`overworld` / `nether` / `end`。

## CI / Release

| Workflow | 触发 | 作用 |
|----------|------|------|
| [CI](.github/workflows/ci.yml) | `push`/`PR` → `main` | `cargo test`、clippy、CLI 冒烟 |
| [Release](.github/workflows/release.yml) | 推送 tag `v*.*.*` | 多平台 release + skill |

产物：`mcaedit-<target>.tar.gz`、`mcaedit-skill.tar.gz`、`install.sh` / `install.ps1`。

```bash
git tag v0.11.1
git push origin v0.11.1
```

## level.dat / 新建世界 / 结构

```bash
# 默认 Minecraft 26.2（DataVersion 4903）；也可用 --mc 1.21.4|1.20.1|1.18.2 或裸 DataVersion
mcaedit world create --path ./myworld --name Demo --seed 42 --mc 26.2 --generator flat
mcaedit level info --world ./myworld
mcaedit level patch --world ./myworld --spawn 0,64,0 --game-type 1 --touch

# world create=新建；session --bootstrap=缺骨架则补齐（已有 level.dat 则复用，不覆盖）
mcaedit session create --world ./myworld --id demo --bootstrap --mc 26.2 --seed 42

# 原版结构 .nbt（与 Sponge .schem 不同）
mcaedit --session demo structure export --from 0,64,0 --to 7,70,7 --out hut.nbt
mcaedit --session demo structure place --file hut.nbt --at 32,64,32 --rotation 90
mcaedit --session demo structure list --world ./myworld
mcaedit --session demo inspect structures --cx 2 --cz 2
mcaedit --session demo structure clear-refs --from 0,0,0 --to 63,0,63
```

支持的 `--mc` 别名：`26.2=4903`，`1.21.11=4671`，`1.21.10=4556`，`1.21.4=4189`，`1.21.1=3955`，`1.21=3738`，`1.20.4=3700`，`1.20.1=3465`，`1.20=3463`，`1.19.4=3337`，`1.18.2=2975`。

## Region edits

选区一律 `--from x,y,z --to x,y,z`（无 pos1/pos2）。负坐标可用 `--from -8,60,-8` 或 `--from=-8,60,-8`。`--block` / `--pattern` 支持 `%` 权重（如 `50%stone,50%dirt`）；`--mask` / `--mask-exclude` 可组合过滤。

```bash
mcaedit --session demo edit fill --from 0,64,0 --to 7,66,7 --block minecraft:stone
mcaedit --session demo edit fill --from 0,64,0 --to 15,64,15 --pattern '50%stone,50%dirt'
mcaedit --session demo edit replace --from 0,64,0 --to 7,66,7 \
  --match air --with minecraft:glass
mcaedit --session demo edit replace --from 0,64,0 --to 15,70,15 \
  --mask 'stone,dirt' --with '70%cobblestone,30%gravel'
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

## Brush / smooth / smooth3d / biome

```bash
mcaedit --session demo edit brush sphere --at 0,70,0 --radius 5 \
  --pattern '50%stone,50%dirt' --mask air
mcaedit --session demo edit brush cyl --at 0,0 --y 64 --radius 4 --height 8 \
  --block minecraft:sand --mask '#solid'
mcaedit --session demo edit brush biome --at 0,70,0 --radius 8 --biome minecraft:desert
mcaedit --session demo edit brush biome-cyl --at 0,0 --y 64 --radius 6 --height 16 \
  --biome plains --mask air
mcaedit --session demo edit copy --from 0,64,0 --to 2,65,2
mcaedit --session demo edit brush clipboard --at 32,64,32 --radius 6 --mask air
mcaedit --session demo edit smooth --from 0,60,0 --to 31,80,31 --iterations 2 --kernel 1
mcaedit --session demo edit smooth3d --from 0,60,0 --to 31,80,31 --iterations 2 --kernel 1
mcaedit --session demo edit smooth3d --from 0,60,0 --to 31,80,31 --iterations 3 --kernel 1 --solid
mcaedit --session demo edit biome --from 0,64,0 --to 31,80,31 --biome minecraft:desert
```

- Biome brush：世界空间球/柱形状，写入 MCA 4×4×4 biome cell；可 undo；可选 `--mask`（按 cell 原点方块过滤）
- `smooth`：heightmap 表面平滑；`smooth3d`：体素邻域多数表决（`--solid` 先投 air/solid）
- Biome paint：AABB 相交的 4×4×4 cell

## Linear region

支持 `region/r.X.Z.linear`（Linear v1 / v2）。Session 打开时把 Linear 转成工作副本 `.mca`；若源为 Linear，`commit` 写回 `.linear`。

## Sponge `.schem`

```bash
mcaedit schem info --file /tmp/box.schem
mcaedit --json schem info --file /tmp/box.schem
mcaedit --session demo schem export --from 0,64,0 --to 15,80,15 --out /tmp/box.schem
mcaedit --session demo schem import --file /tmp/box.schem --at 64,64,64
mcaedit template export-schem --name hut --out /tmp/hut.schem
mcaedit template import-schem --file /tmp/hut.schem --name hut
```

写出 Sponge Schematic **v2**（根下 `Schematic` 包装，兼容 WE/FAWE）；读取支持 **v1/v2/v3**。`info` 给出 DataVersion、Offset、volume、方块 Top-N。`import` 放置原点 = `--at` + Offset。不支持经典 MCEdit `.schematic`。

## Offline view screenshot / live preview

`view screenshot` 会从 session 工作副本的选区构建可见面网格并离线渲成 PNG，适合直接给 LLM 读图（不依赖 Node/dotnet/浏览器）。

```bash
mcaedit --session demo view screenshot \
  --from 0,64,0 --to 31,80,31 \
  --out /tmp/shot.png \
  --width 1280 --height 720
```

可选：`--camera x,y,z` 与 `--look x,y,z` 覆盖默认取景。

**方块模型 + 贴图（Minecraft 26.2 client jar）**：读 `assets/minecraft/blockstates/*.json` + `models/block/*.json`（variants / multipart）与 `textures/block/*.png`；软光栅按模型 UV 最近邻采样。缺 jar 时回退调色板立方体（不崩溃）。

**光照**：采样 section `BlockLight`/`SkyLight` × lightmap 曲线 × 六面 shade，外加简化顶点 AO。室内/火把效果依赖世界已有光数据（可用 `edit fix-light`）。

```bash
# 显式指定 client jar，或 versions/26.2/ 目录
mcaedit view screenshot --from 0,64,0 --to 15,80,15 --out /tmp/shot.png \
  --minecraft ~/.minecraft/versions/26.2/26.2.jar
# 等价环境变量（任选其一）
export MCAEDIT_MINECRAFT_JAR=~/.minecraft/versions/26.2/26.2.jar
# export MCAEDIT_ASSETS_JAR=...
# 强制纯色
mcaedit view screenshot ... --no-textures
```

未指定时自动探测：`/other/Minecraft/.minecraft/versions/26.2/26.2.jar`、`~/.minecraft/...`、Flatpak、Prism/PolyMC/MultiMC、HMCL、`%APPDATA%/.minecraft/...`。环境变量 `MCAEDIT_MINECRAFT_JAR` / `MCAEDIT_ASSETS_JAR`。日志形如 `models+textures jar path=...` 或 `palette reason=...`。
大体积截图默认上限 2e6 cells（`--max-cells` / `MCAEDIT_VIEW_MAX_CELLS`）。仓库**不**内嵌整包 jar。

**`view preview` / `preview`** 打开原生窗口，轮询 session 工作副本 `region/*.mca`、`meta.json`、`HEAD`，在另一终端跑 `edit fill/brush/...` 时可看实时建造进度（与 screenshot 共用 model/mesh/贴图/光照管线）。

```bash
# 终端 A：预览（需要 DISPLAY 或 Wayland）
mcaedit --session demo view preview --from 0,64,0 --to 31,80,31 --watch 400
# 或省略 --from/--to，按工作区 chunk 自动裁到 ≤48³
mcaedit --session demo preview --watch 500 --minecraft ~/.minecraft/versions/26.2

# 终端 B：边建边看
mcaedit --session demo edit fill --from 0,64,0 --to 15,70,15 --pattern '50%stone,50%dirt'
mcaedit --session demo edit brush sphere --at 8,72,8 --radius 4 --block minecraft:glass
```

操作：LMB 轨道旋转、RMB/中键平移、滚轮缩放、空格切换 auto-orbit。无显示器的 CI/SSH 不要开窗口；cargo feature `preview` 默认开启（`--no-default-features` 可关掉 GUI 依赖）。

Clipboard（会话内）：

```bash
mcaedit --session demo edit copy --from 0,64,0 --to 3,65,2
mcaedit --session demo edit rotate --yaw 90
mcaedit --session demo edit flip --axis x
mcaedit --session demo edit paste --at 32,64,32
mcaedit --session demo edit cut --from 0,64,0 --to 3,65,2
```

对照表见 skill：`.codex/skills/mcaedit/references/region-ops.md`。本功能的离线取景/渲染思路参考 [Arcus92/minecraft-web-viewer](https://github.com/Arcus92/minecraft-web-viewer) 与 [Arcus92/minecraft-web-exporter](https://github.com/Arcus92/minecraft-web-exporter)（MIT）。

已知限制：biome 为 section 内 4×4×4；离线 tick 为近似生长 + scheduled 队列步进（非完整服务端）；Linear 工作副本以 Anvil 编辑后按源格式写回；view 已加载 vanilla blockstates/models + BlockLight/SkyLight（参考 Arcus92/minecraft-web-exporter）；无 CTM/流体曲面/实体方块特殊渲染；动画贴图仅首帧；AO 为简化顶点遮挡。
## Multi-session collaboration

```bash
mcaedit session create --world /path/to/world --id alice --label agent-a
mcaedit session create --world /path/to/world --id bob --label agent-b
mcaedit session list
mcaedit --session alice edit set-block --x 0 --y 64 --z 0 --block minecraft:stone
mcaedit --session alice commit
mcaedit --session bob session sync
```

Working copies: `./.mcaedit/<id>/`（含 `clipboard.json`）。Templates: `./.mcaedit/templates/`。

## Inspect

```bash
mcaedit inspect summary --cx 0 --cz 0
mcaedit inspect get --x 1 --y 64 --z 2
mcaedit inspect slice --y 64 --from 0,0 --to 8,8
mcaedit inspect select --from 0,64,0 --to 7,66,7
mcaedit inspect palette --cx 0 --cz 0 --sy 4
mcaedit inspect entities --near 0,64,0 --r 32
```

## Templates

```bash
mcaedit --session alice template save --name hut --from 0,64,0 --to 5,67,5
mcaedit template list
mcaedit --session bob template paste --name hut --at 32,64,32
```

## History / commit

```bash
mcaedit history list
mcaedit history undo --n 1
mcaedit history redo --n 1
mcaedit history revert --to 0
mcaedit commit
mcaedit commit --dry-run
```

## Notes

- Supports `region/` + `entities/` `.mca` and Linear `.linear` (v1/v2).
- Heightmaps/lighting are not recalculated (except via `edit fix-light`).
- Block states are named strings; no full registry validation.
- `--json` on selected commands for machine output.
- Env: `MCAEDIT_SESSION=<id>`.
- Windows / large MCA writes need a large stack; the CLI spawns a 16MiB worker.


## 建造助手 / 大盒验收 / 租约（v0.9）

```bash
mcaedit edit grid --from 0,64,0 --to 31,72,31 --spacing-x 4 --spacing-z 4 --block minecraft:oak_log
mcaedit edit roof-rows --from 0,80,0 --to 31,80,31 --axis z --period 2 --block minecraft:brick_slab
mcaedit edit stairs --from 0,64,0 --to 7,64,0 --block minecraft:oak_stairs --facing east
mcaedit inspect summary-box --from=-10,55,-55 --to=45,100,12
mcaedit session lease --from=-32,60,-32 --to=32,90,32
```
