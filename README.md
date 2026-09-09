# MCAEdit

Offline Minecraft Anvil (`.mca`) editor CLI for LLMs — **GPL-3.0**（含地形生成 / 光照）。

仓库：[CntierTeam/MCAEdit](https://github.com/CntierTeam/MCAEdit)

Flow: `session create` → `inspect` / `edit` / `view screenshot`（区域编辑 + **gen / fix-light / tick-participate**）→ `history` → `commit`

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
  | bash -s -- --version v0.4.0 --force
./scripts/install.sh --from-source --symlink-skill --force
./scripts/install.sh --uninstall
```

## 从源码构建

地形栈在仓库内 submodule：`vendor/pumpkin`（`scripts/ensure-vendor.sh` 会 init/浅克隆，并修补嵌套 workspace 继承）。

```bash
bash scripts/ensure-vendor.sh
rustup default stable
cargo build --release -p mcaedit-cli
./target/release/mcaedit --help
```

或：`./scripts/install.sh --from-source --force`（内部会先 ensure-vendor）。

## 地形 / 光照 / tick

坐标为 **chunk**（`--from x,z --to x,z`）：

```bash
mcaedit --session demo edit gen --seed 42 --dim overworld --from 0,0 --to 3,3
mcaedit --session demo edit fix-light --from 0,0 --to 3,3 --seed 42 --dim overworld
mcaedit --session demo edit tick-participate --from 0,0 --to 3,3 --rounds 20 --speed 3
```

- `gen`：Full 阶段写入 session 工作副本 `region/`
- `fix-light`：重算天空/方块光
- `tick-participate`：重建 random-tick mask，采样候选，步进 `block_ticks`/`fluid_ticks`（完整作物等行为需服务端）

`--dim`：`overworld` / `nether` / `end`。

## CI / Release

| Workflow | 触发 | 作用 |
|----------|------|------|
| [CI](.github/workflows/ci.yml) | `push`/`PR` → `main` | `cargo test`、clippy、CLI 冒烟 |
| [Release](.github/workflows/release.yml) | 推送 tag `v*.*.*` | 多平台 release + skill |

产物：`mcaedit-<target>.tar.gz`、`mcaedit-skill.tar.gz`、`install.sh` / `install.ps1`。

```bash
git tag v0.4.0
git push origin v0.4.0
```

## Region edits

选区一律 `--from x,y,z --to x,y,z`（无 pos1/pos2）。`--block` / `--pattern` 支持 `%` 权重（如 `50%stone,50%dirt`）；`--mask` / `--mask-exclude` 可组合过滤。

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

## Brush / smooth / biome

```bash
mcaedit --session demo edit brush sphere --at 0,70,0 --radius 5 \
  --pattern '50%stone,50%dirt' --mask air
mcaedit --session demo edit brush cyl --at 0,0 --y 64 --radius 4 --height 8 \
  --block minecraft:sand --mask '#solid'
mcaedit --session demo edit copy --from 0,64,0 --to 2,65,2
mcaedit --session demo edit brush clipboard --at 32,64,32 --radius 6 --mask air
mcaedit --session demo edit smooth --from 0,60,0 --to 31,80,31 --iterations 2 --kernel 1
mcaedit --session demo edit biome --from 0,64,0 --to 31,80,31 --biome minecraft:desert
```

Biome 按 MCA section 的 4×4×4 分辨率写入，可 undo。

## Sponge `.schem`

```bash
mcaedit --session demo schem export --from 0,64,0 --to 15,80,15 --out /tmp/box.schem
mcaedit schem info --file /tmp/box.schem
mcaedit --session demo schem import --file /tmp/box.schem --at 64,64,64
mcaedit template export-schem --name hut --out /tmp/hut.schem
mcaedit template import-schem --file /tmp/hut.schem --name hut
```

写出 Sponge Schematic **v2**；读取支持 v2/v3。

## Offline view screenshot

`view screenshot` 会从 session 工作副本的选区构建可见面网格并离线渲成 PNG，适合直接给 LLM 读图（不依赖 Node/dotnet/浏览器）。

```bash
mcaedit --session demo view screenshot \
  --from 0,64,0 --to 31,80,31 \
  --out /tmp/shot.png \
  --width 1280 --height 720
```

可选：`--camera x,y,z` 与 `--look x,y,z` 覆盖默认取景。

Clipboard（会话内）：

```bash
mcaedit --session demo edit copy --from 0,64,0 --to 3,65,2
mcaedit --session demo edit rotate --yaw 90
mcaedit --session demo edit flip --axis x
mcaedit --session demo edit paste --at 32,64,32
mcaedit --session demo edit cut --from 0,64,0 --to 3,65,2
```

对照表见 skill：`.codex/skills/mcaedit/references/region-ops.md`。本功能的离线取景/渲染思路参考 [Arcus92/minecraft-web-viewer](https://github.com/Arcus92/minecraft-web-viewer) 与 [Arcus92/minecraft-web-exporter](https://github.com/Arcus92/minecraft-web-exporter)（MIT）。

已知限制：biome 为 section 内 4×4×4；离线 tick 非完整服务端行为；无 Linear region。
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

- Supports standard `region/` + `entities/` `.mca` only (not Linear region formats).
- Heightmaps/lighting are not recalculated.
- Block states are named strings; no full registry validation.
- `--json` on selected commands for machine output.
- Env: `MCAEDIT_SESSION=<id>`.
- Windows / large MCA writes need a large stack; the CLI spawns a 16MiB worker.
