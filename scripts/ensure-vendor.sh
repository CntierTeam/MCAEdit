#!/usr/bin/env bash
# Ensure vendor/pumpkin exists for path deps (mixed compile with worldgen stack).
# Also patches package.workspace so Cargo does not inherit from MCAEdit's outer
# workspace (nested workspace / rust-lang/cargo#12154).
#
# Pumpkin prune (裁枝): by default, rewrite pumpkin-world → pumpkin-data features so
# MCAEdit only compiles offline worldgen/lighting data modules — not server/protocol
# remaps, items, translations, advancements, bedrock tables, etc.
# Full vendor clone is still required (path layout + workspace manifests); only the
# *compile* graph is pruned. Set MCAEDIT_PUMPKIN_MINIMAL=0 to keep Pumpkin defaults.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENDOR="${ROOT}/vendor/pumpkin"
URL="${MCAEDIT_VENDOR_URL:-https://github.com/Pumpkin-MC/Pumpkin.git}"
REF="${MCAEDIT_VENDOR_REF:-master}"
# Default ON: prune pumpkin-data features for MCAEdit offline terrain/light.
MINIMAL="${MCAEDIT_PUMPKIN_MINIMAL:-1}"

ready() {
  [[ -f "${VENDOR}/Cargo.toml" && -f "${VENDOR}/crates/pumpkin-world/Cargo.toml" ]]
}

# Point each package at Pumpkin's workspace root so .workspace = true resolves
# correctly when this tree sits under MCAEdit's workspace directory.
patch_nested_workspace() {
  python3 - <<'PY' "${VENDOR}"
import os, re, sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
count = 0
for manifest in root.rglob("Cargo.toml"):
    if manifest.resolve() == (root / "Cargo.toml").resolve():
        continue
    text = manifest.read_text(encoding="utf-8")
    if "workspace = true" not in text and ".workspace = true" not in text:
        continue
    pkg_start = text.find("[package]")
    if pkg_start < 0:
        continue
    next_hdr = len(text)
    for m in re.finditer(r"\n\[\[|\n\[", text[pkg_start + 1 :]):
        next_hdr = pkg_start + 1 + m.start()
        break
    pkg = text[pkg_start:next_hdr]
    if re.search(r"(?m)^workspace\s*=", pkg):
        continue
    rel = os.path.relpath(root, manifest.parent).replace("\\", "/")
    rest = text[pkg_start + len("[package]") :]
    if rest.startswith("\n"):
        rest = rest[1:]
    manifest.write_text(f'[package]\nworkspace = "{rel}"\n' + rest, encoding="utf-8")
    count += 1
print(f"patched package.workspace on {count} manifests under vendor/pumpkin")
PY
}

# Features pumpkin-world actually needs for gen / fix-light / tick (see terrain_bridge).
# Explicitly excludes: item, packet, translation, registry, recipes, advancement,
# entity, sound, bedrock_*, tracked_data, data_component, potion, villager, …
WORLDGEN_DATA_FEATURES=(
  block
  chunk
  dimension
  fluid
  game_rules
  tag
  noise_router
  material_rule
  noise_settings
  carver
  structures
  placed_feature
  configured_feature
)

prune_pumpkin_data_features() {
  local feats_csv
  feats_csv="$(IFS=,; echo "${WORLDGEN_DATA_FEATURES[*]}")"
  MCAEDIT_PRUNE_FEATURES="${feats_csv}" python3 - <<'PY' "${VENDOR}" "${MINIMAL}"
import os, re, sys
from pathlib import Path

vendor = Path(sys.argv[1]).resolve()
minimal = sys.argv[2].strip() not in ("0", "false", "False", "no", "NO")
features = [f for f in os.environ.get("MCAEDIT_PRUNE_FEATURES", "").split(",") if f]

world_toml = vendor / "crates/pumpkin-world/Cargo.toml"
data_toml = vendor / "crates/pumpkin-data/Cargo.toml"
data_lib = vendor / "crates/pumpkin-data/src/lib.rs"
marker = vendor / ".mcaedit-pumpkin-pruned"

world_text = world_toml.read_text(encoding="utf-8")
dep_pat = re.compile(r"(?m)^pumpkin-data\s*=\s*\{[^}]*\}")

if not minimal:
    replacement = 'pumpkin-data = { workspace = true, features = ["default"] }'
    new_world, n = dep_pat.subn(replacement, world_text, count=1)
    if n:
        world_toml.write_text(new_world, encoding="utf-8")
    if marker.exists():
        marker.unlink()
    print("pumpkin prune: OFF (MCAEDIT_PUMPKIN_MINIMAL=0) — pumpkin-data default features")
    sys.exit(0)

feat_list = ", ".join(f'"{f}"' for f in features)
replacement = (
    f"pumpkin-data = {{ workspace = true, default-features = false, "
    f"features = [{feat_list}] }}"
)
new_world, n = dep_pat.subn(replacement, world_text, count=1)
if n == 0:
    print("error: could not find pumpkin-data dep in pumpkin-world/Cargo.toml", file=sys.stderr)
    sys.exit(1)
world_toml.write_text(new_world, encoding="utf-8")

# Gate always-on loot_table (server-only; ~1.2M generated) behind a feature.
data_toml_text = data_toml.read_text(encoding="utf-8")
if "loot_table = []" not in data_toml_text:
    data_toml_text = data_toml_text.replace(
        "\ntrial_spawner = []\n",
        "\ntrial_spawner = []\nloot_table = []\n",
        1,
    )
    if "loot_table = []" not in data_toml_text:
        data_toml_text = data_toml_text.replace(
            "\n[dependencies]\n",
            "\nloot_table = []\n\n[dependencies]\n",
            1,
        )
    data_toml.write_text(data_toml_text, encoding="utf-8")

lib_text = data_lib.read_text(encoding="utf-8")
old_loot = (
    "#[rustfmt::skip]\n"
    '#[path = "generated/loot_table.rs"]\n'
    "pub mod loot_table;\n"
    "pub use loot_table as chest_loot_table;\n"
)
new_loot = (
    '#[cfg(feature = "loot_table")]\n'
    "#[rustfmt::skip]\n"
    '#[path = "generated/loot_table.rs"]\n'
    "pub mod loot_table;\n"
    '#[cfg(feature = "loot_table")]\n'
    "pub use loot_table as chest_loot_table;\n"
)
if '#[cfg(feature = "loot_table")]\n#[rustfmt::skip]\n#[path = "generated/loot_table.rs"]' not in lib_text:
    if old_loot not in lib_text:
        print("warning: loot_table block not found for cfg gate; skipping", file=sys.stderr)
    else:
        data_lib.write_text(lib_text.replace(old_loot, new_loot, 1), encoding="utf-8")

# biome.rs imports EntityType but never uses it; keeping the import would force the
# entity_type → sound → attributes → data_component → item cascade (~10MB+).
biome_gen = vendor / "crates/pumpkin-data/src/generated/biome.rs"
if biome_gen.is_file():
    biome_text = biome_gen.read_text(encoding="utf-8")
    stripped, n_biome = re.subn(
        r"(?m)^use crate::entity_type::EntityType;\n",
        "",
        biome_text,
        count=1,
    )
    if n_biome:
        biome_gen.write_text(stripped, encoding="utf-8")
        print("pumpkin prune: stripped unused EntityType import from biome.rs")

# end_city only needs Item/ItemStack to embed an elytra into an item-frame NBT.
# Replacing that with a hand-written compound avoids enabling the entire item graph.
end_city = vendor / "crates/pumpkin-world/src/generation/structure/structures/end_city/mod.rs"
if end_city.is_file():
    ec = end_city.read_text(encoding="utf-8")
    ec2 = ec.replace(
        "use pumpkin_data::{BlockDirection, Mirror, Rotation, item::Item, item_stack::ItemStack};",
        "use pumpkin_data::{BlockDirection, Mirror, Rotation};",
    )
    old_elytra = (
        "        let stack = ItemStack::new(1, &Item::ELYTRA);\n"
        "        let mut item = NbtCompound::new();\n"
        "        stack.write_item_stack(&mut item);\n"
        "        nbt.put_compound(\"Item\", item);\n"
    )
    new_elytra = (
        "        // MCAEdit prune: avoid pumpkin-data `item` feature (elytra NBT only).\n"
        "        let mut item = NbtCompound::new();\n"
        "        item.put_string(\"id\", \"minecraft:elytra\".into());\n"
        "        item.put_byte(\"count\", 1);\n"
        "        nbt.put_compound(\"Item\", item);\n"
    )
    if old_elytra in ec2:
        ec2 = ec2.replace(old_elytra, new_elytra, 1)
        end_city.write_text(ec2, encoding="utf-8")
        print("pumpkin prune: end_city elytra NBT no longer needs pumpkin-data item feature")
    elif "MCAEdit prune: avoid pumpkin-data `item` feature" in ec2:
        if ec2 != ec:
            end_city.write_text(ec2, encoding="utf-8")
    elif "item::Item" in ec:
        print("warning: end_city Item usage shape changed; patch manually", file=sys.stderr)

# Aggregate feature for docs / MCAEdit Cargo.toml mirroring.
data_toml_text = data_toml.read_text(encoding="utf-8")
agg = "mcaedit-worldgen = [" + ", ".join(f'"{f}"' for f in features) + "]"
if "mcaedit-worldgen" not in data_toml_text:
    data_toml_text = data_toml_text.replace(
        "\nloot_table = []\n",
        "\nloot_table = []\n" + agg + "\n",
        1,
    )
    if "mcaedit-worldgen" not in data_toml_text:
        data_toml_text = data_toml_text.replace(
            "\n[dependencies]\n",
            "\n" + agg + "\n\n[dependencies]\n",
            1,
        )
    data_toml.write_text(data_toml_text, encoding="utf-8")

marker.write_text(
    "MCAEdit pumpkin-data prune applied by scripts/ensure-vendor.sh\n"
    f"features={','.join(features)}\n"
    "excluded_examples=item,translation,registry,advancement,recipes,entity,sound,"
    "bedrock_*,tracked_data,packet,loot_table,villager,potion,…\n"
    "note: full vendor clone still needed; server/protocol/plugin crates are not "
    "linked by MCAEdit path deps even before prune.\n",
    encoding="utf-8",
)
print(
    f"pumpkin prune: ON — pumpkin-data features=[{', '.join(features)}] "
    f"(+ loot_table cfg-gated off)"
)
print(
    "  still compiled: block/biome/tag/noise/structures (worldgen core); "
    "not linked: pumpkin server/protocol/plugins"
)
PY
}

fetch_vendor() {
  mkdir -p "${ROOT}/vendor"
  local tmp
  tmp="$(mktemp -d "${ROOT}/vendor/.pumpkin-fetch.XXXXXX")"
  cleanup_tmp() { rm -rf "${tmp}" 2>/dev/null || true; }
  trap cleanup_tmp EXIT

  if [[ -f "${ROOT}/.gitmodules" ]] && command -v git >/dev/null 2>&1; then
    if git -C "${ROOT}" submodule update --init --depth 1 vendor/pumpkin 2>/dev/null && ready; then
      echo "vendor/pumpkin ready (submodule)"
      trap - EXIT
      cleanup_tmp
      return 0
    fi
  fi

  echo "cloning ${URL} (${REF}) into vendor/pumpkin …"
  if ! git clone --depth 1 --branch "${REF}" "${URL}" "${tmp}/pumpkin"; then
    echo "error: git clone failed" >&2
    exit 1
  fi
  if [[ -e "${VENDOR}" ]]; then
    local aside
    aside="${ROOT}/vendor/.pumpkin-aside.$$"
    mv "${VENDOR}" "${aside}" 2>/dev/null || true
    mkdir -p "${VENDOR}"
    cp -a "${tmp}/pumpkin/." "${VENDOR}/"
    rm -rf "${aside}" 2>/dev/null || true
  else
    mv "${tmp}/pumpkin" "${VENDOR}"
  fi
  trap - EXIT
  cleanup_tmp
  ready || {
    echo "error: vendor/pumpkin incomplete after clone" >&2
    exit 1
  }
  echo "vendor/pumpkin ready (clone)"
}

if ! ready; then
  fetch_vendor
fi

patch_nested_workspace
prune_pumpkin_data_features
