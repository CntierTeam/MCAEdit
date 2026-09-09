#!/usr/bin/env bash
# Ensure vendor/pumpkin exists for path deps (mixed compile with worldgen stack).
# Also patches package.workspace so Cargo does not inherit from MCAEdit's outer
# workspace (nested workspace / rust-lang/cargo#12154).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENDOR="${ROOT}/vendor/pumpkin"
URL="${MCAEDIT_VENDOR_URL:-https://github.com/Pumpkin-MC/Pumpkin.git}"
REF="${MCAEDIT_VENDOR_REF:-master}"

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

if ! ready; then
  mkdir -p "${ROOT}/vendor"
  rm -rf "${VENDOR}"

  if [[ -f "${ROOT}/.gitmodules" ]] && command -v git >/dev/null 2>&1; then
    if git -C "${ROOT}" submodule update --init --depth 1 vendor/pumpkin 2>/dev/null; then
      if ready; then
        echo "vendor/pumpkin ready (submodule)"
      fi
    fi
  fi

  if ! ready; then
    rm -rf "${VENDOR}"
    echo "cloning ${URL} (${REF}) into vendor/pumpkin …"
    git clone --depth 1 --branch "${REF}" "${URL}" "${VENDOR}"
    ready || {
      echo "error: vendor/pumpkin incomplete after clone" >&2
      exit 1
    }
    echo "vendor/pumpkin ready (clone)"
  fi
fi

patch_nested_workspace
