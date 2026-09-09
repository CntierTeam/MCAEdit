# MCAEdit

Offline Minecraft Anvil (`.mca`) editor CLI for LLMs — **GPL-3.0**（链接 [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) 地形生成 / 光照）。

仓库：[CntierTeam/MCAEdit](https://github.com/CntierTeam/MCAEdit)

Flow: `session create` → `inspect` / `edit`（WorldEdit 子集 + **gen / fix-light / tick-participate**）→ `history` → `commit`

本仓库同时提供 **Codex Skill**（`$mcaedit`）：**操作员代跑 / execute-first**——在 shell 直接跑 `mcaedit`（session / inspect / edit / commit），不是只拼命令。

## 许可

本项目与 Pumpkin 相同，使用 **GNU GPL v3**。分发二进制时须提供对应源码。

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
  | bash -s -- --version v0.3.0 --force
./scripts/install.sh --from-source --symlink-skill --force
./scripts/install.sh --uninstall
```

## 从源码构建

需在**同级目录**克隆 Pumpkin（`Cargo.toml` 依赖 `../Pumpkin/crates/pumpkin-*`）：

```bash
cd .. # IdeaProjects /
git clone --depth 1 https://github.com/Pumpkin-MC/Pumpkin.git Pumpkin
cd MCAEdit
rustup default stable
cargo build --release -p mcaedit-cli
./target/release/mcaedit --help
```

## Pumpkin：地形 / 光照 / tick

坐标为 **chunk**（`--from x,z --to x,z`）：

```bash
mcaedit --session demo edit gen --seed 42 --dim overworld --from 0,0 --to 3,3
mcaedit --session demo edit fix-light --from 0,0 --to 3,3 --seed 42 --dim overworld
mcaedit --session demo edit tick-participate --from 0,0 --to 3,3 --rounds 20 --speed 3
```

- `gen`：Pumpkin Full 阶段写入 session 工作副本 `region/`
- `fix-light`：`LightEngine::initialize_light` 重算天空/方块光
- `tick-participate`：重建 random-tick mask，采样候选，步进 `block_ticks`/`fluid_ticks`（完整作物等行为在 Pumpkin 服务端）

`--dim`：`overworld` / `nether` / `end`。

## CI / Release

| Workflow | 触发 | 作用 |
|----------|------|------|
| [CI](.github/workflows/ci.yml) | `push`/`PR` → `main` | `cargo test`、clippy、CLI 冒烟 |
| [Release](.github/workflows/release.yml) | 推送 tag `v*.*.*` | 多平台 release + skill |

产物：`mcaedit-<target>.tar.gz`、`mcaedit-skill.tar.gz`、`install.sh` / `install.ps1`。

```bash
git tag v0.2.0
git push origin v0.2.0
```

## WorldEdit-style edits

选区一律 `--from x,y,z --to x,y,z`（无 pos1/pos2）。

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

Clipboard（会话内）：

```bash
mcaedit --session demo edit copy --from 0,64,0 --to 3,65,2
mcaedit --session demo edit rotate --yaw 90
mcaedit --session demo edit flip --axis x
mcaedit --session demo edit paste --at 32,64,32
mcaedit --session demo edit cut --from 0,64,0 --to 3,65,2
```

对照表见 skill：`.codex/skills/mcaedit/references/worldedit-map.md`。

**未实现：** brush、复杂 mask、百分比 pattern、`//smooth`、生物群系、`.schem` 互通。

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
