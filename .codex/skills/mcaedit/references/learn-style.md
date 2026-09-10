# `/learn` / 标注 — 从 schem/mca 提炼建筑风格子 SKILL

主入口见 [SKILL.md](../SKILL.md)「风格学习 `/learn`」。本文件是完整步骤。

## 触发

用户说：`/learn`、`标注`、`learn style from …`、`从 xxx.schem 学风格`。

## 输入

| 输入 | 做法 |
|------|------|
| `*.schem` | `mcaedit --json schem info --file PATH --style-hints`（无需 session） |
| 世界 / `.mca` + AABB | `session create` → `inspect summary-box --from/--to`；可选 `schem export` 再 `--style-hints` |
| 当前 session AABB | 同上，或先 `schem export` 再分析 |

## 步骤清单

```
Task Progress:
- [ ] 1. 取名：styles/<slug>/（小写连字符，如 song-timber、oak-frame）
- [ ] 2. 跑 CLI 拿 JSON / style_hints
- [ ] 3. 提炼材质 / 层 / 楼梯朝向 / 柱距
- [ ] 4. 写成 styles/<slug>/SKILL.md（复制 _template）
- [ ] 5. 在主 SKILL「已学风格」表加一行（可选）
- [ ] 6. 告诉用户：以后 /use style <slug> 或读该 SKILL.md
```

### 1–2. 采集

```bash
# .schem（推荐）
mcaedit --json schem info --file /path/to/build.schem --style-hints

# 世界 AABB
mcaedit session create --world /path/to/world --id learn --label agent
export MCAEDIT_SESSION=learn
mcaedit inspect summary-box --from=-10,55,-55 --to=45,100,12
# 可选：导出再分析
mcaedit schem export --from=-10,55,-55 --to=45,100,12 --out /tmp/learn.schem
mcaedit --json schem info --file /tmp/learn.schem --style-hints
```

`style_hints` 字段：`materials_top`、`families`、`stairs_facing` / `stairs_half`、`slab_type`、`layers[]`、`pillar_spacing_hint`、`suggested_ops`、`solid_ratio`。

### 3. 提炼要点

- **主材**：去掉 air；按 families 分结构/填充/屋顶/装饰
- **竖向分层**：`layers[].dominant` → 地基 / 墙身 / 檐口 / 屋顶
- **楼梯**：dominant `facing` + `half` → `edit stairs` / `roof-rows --stairs-facing`
- **柱网**：`pillar_spacing_hint` → `edit grid` / `colonnade --spacing-x/z`
- **节奏**：瓦垄 period、柱距、开间宽度（量 AABB 或从 hint 推）

### 4. 写子 SKILL

路径：`.codex/skills/mcaedit/styles/<slug>/SKILL.md`

复制 [styles/_template/SKILL.md](../styles/_template/SKILL.md)，填：

1. frontmatter `name` / `description`（含触发词）
2. 材质表
3. 建造菜谱（只用已有 ops：`fill` `walls` `grid` `colonnade` `roof-rows` `stairs` `schem import` …）
4. DO / DON'T
5. 可复制的示例命令（负坐标用 `=`）

`disable-model-invocation: true`：子风格默认不自动灌上下文，由 `/use style` 或主 skill 指引再读。

### 5. 复用

用户：`/use style <slug>`、`按 oak-frame 风格建`、`用上次标注的风格`。

Agent：

1. `Read` `.codex/skills/mcaedit/styles/<slug>/SKILL.md`
2. 按其中菜谱用 `mcaedit` 代跑
3. 不要发明模板外的方块 id；缺路径/坐标再问一句

## 命名

- slug：`[a-z0-9-]+`，短、可搜
- 避免：`style1`、`tmp`、`test`
- 示例：`oak-frame`、`song-timber`、`nether-brick-keep`

## 诚实边界

- `--style-hints` 是启发式，不是完整建筑语义分割
- 柱距 / 瓦垄可能误判；以 `inspect` / 截图复核
- 复用时优先「同比例重建菜谱」，精确复刻用 `schem import`
