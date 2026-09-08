# MCAEdit

Offline Minecraft Anvil (`.mca`) editor CLI for LLMs.

仓库：[CntierTeam/MCAEdit](https://github.com/CntierTeam/MCAEdit)

Flow: `session create` → `inspect` / `edit` → `history undo|redo|revert` → `commit`

本仓库同时提供 **Codex Skill**（`$mcaedit`）。

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

常用选项：

```bash
# 指定版本
curl -fsSL https://raw.githubusercontent.com/CntierTeam/MCAEdit/main/scripts/install.sh \
  | bash -s -- --version v0.1.0 --force

# 只装二进制 / 只装 skill
bash scripts/install.sh --bin-only
bash scripts/install.sh --skill-only --force

# 开发机：从本地源码安装
./scripts/install.sh --from-source --symlink-skill --force

# 卸载
./scripts/install.sh --uninstall
```

## 从源码构建

```bash
rustup default stable
cargo build --release -p mcaedit-cli
./target/release/mcaedit --help
```

## CI / Release

| Workflow | 触发 | 作用 |
|----------|------|------|
| [CI](.github/workflows/ci.yml) | `push`/`PR` → `main` | `cargo test`、clippy、CLI 冒烟 |
| [Release](.github/workflows/release.yml) | 推送 tag `v*.*.*` | 多平台 release 构建并发布 GitHub Release |

Release 产物：

- `mcaedit-<target>.tar.gz` — 预编译二进制（linux x86_64/aarch64、macOS aarch64、**Windows x86_64**）
- `mcaedit-skill.tar.gz` — Codex skill
- `install.sh` / `install.ps1` — 安装脚本
- 对应 `.sha256`

打 tag 发版：

```bash
git tag v0.1.0
git push origin v0.1.0
```

## Multi-session collaboration

Multiple agents/users each get their own session on the same world. Commits take an exclusive world lock.

```bash
mcaedit session create --world /path/to/world --id alice --label agent-a
mcaedit session create --world /path/to/world --id bob --label agent-b
mcaedit session list
mcaedit --session alice edit set-block --x 0 --y 64 --z 0 --block minecraft:stone
mcaedit --session alice commit
mcaedit --session bob session sync
```

Working copies: `./.mcaedit/<id>/`. Templates: `./.mcaedit/templates/`.

## Inspect

```bash
mcaedit inspect summary --cx 0 --cz 0
mcaedit inspect get --x 1 --y 64 --z 2
mcaedit inspect slice --y 64 --from 0,0 --to 8,8
mcaedit inspect select --from 0,64,0 --to 7,66,7
# palette id table first, then ASCII 3D layers of those ids
mcaedit inspect palette --cx 0 --cz 0 --sy 4
mcaedit inspect entities --near 0,64,0 --r 32
```

## Templates

```bash
mcaedit --session alice template save --name hut --from 0,64,0 --to 5,67,5
mcaedit template list
mcaedit template show --name hut
mcaedit --session bob template paste --name hut --at 32,64,32
mcaedit template rm --name hut
```

## Edit

```bash
mcaedit edit set-block --x 1 --y 64 --z 2 --block minecraft:stone
mcaedit edit fill --from 0,64,0 --to 3,64,3 --block minecraft:dirt
mcaedit edit set-section --cx 0 --cy 4 --cz 0 --file diff.json
mcaedit edit entity spawn --file entity.json
mcaedit edit entity rm --uuid <uuid> --x 1 --z 2
```

`set-section` JSON (`minecraft:void_air` = leave unchanged):

```json
{"cells":{"0,0,0":"minecraft:stone","1,0,0":"minecraft:void_air"}}
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
