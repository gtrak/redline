#!/usr/bin/env bash
# Materializes the patched iocraft tree at vendor/iocraft/ (plan 013-02).
#
# The repo carries the PATCH, not the tree: the pristine iocraft 0.9.1
# .crate and the patch file (both sha256 pinned below) deterministically
# reproduce vendor/iocraft/. That tree is gitignored — on a fresh clone,
# run this script once before the first build (documented in the README),
# and the gate's build stage re-verifies it (`check`), so a missing or
# stale tree fails the gate loudly instead of dying cryptically inside
# cargo's resolution of the [patch.crates-io] path.
#
# The pins are the guard. iocraft is pinned at 0.9.1: if the crate is ever
# bumped, its .crate will not match CRATE_SHA256 (or the patch will not
# apply) and this script refuses to proceed, loudly. A modified patch is
# caught the same way (PATCH_SHA256). That is the upgrade liability
# recorded in 013-02 — every iocraft bump must re-generate (or
# retire) the patch against the new source and update the pin, or the
# build stops here. A version bump must never silently mis-apply a patch.
#
# The patch itself is a unified diff, generated (never hand-written) as
# `diff -ruN <pristine 0.9.1> <patched tree>` over six files:
# Cargo.toml (standalone [workspace] root), src/backend/crossterm.rs
# (the end_frame emit + its byte-exact test), src/backend/mod.rs (the
# trait seam), src/hooks/mod.rs, src/hooks/use_cursor_position.rs (new),
# and src/terminal.rs (the passthrough). It applies from the extracted
# crate root with `git apply -p1` (the tool used here) or `patch -p1`.
#
# Usage:
#   tools/apply-iocraft-patch.sh          # apply: re-materialize the tree
#   tools/apply-iocraft-patch.sh check    # verify the existing tree
set -euo pipefail
cd "$(dirname "$0")/.."
readonly ROOT="$(pwd)"

readonly CRATE_VERSION="0.9.1"
readonly CRATE_SHA256="41b8c1ddc08a912247397bef7b99f9ed409eecc711557f5523fe362d880d6f98"
readonly PATCH_SHA256="f2e569abaae81980dd9723611d120e2035975504394576b7a3d493f604332484"
readonly PATCH="patches/iocraft-${CRATE_VERSION}-use_cursor_position.patch"
readonly DEST="vendor/iocraft"
readonly DOWNLOAD_URL="https://static.crates.io/crates/iocraft/iocraft-${CRATE_VERSION}.crate"

die() { printf 'apply-iocraft-patch: ERROR: %s\n' "$*" >&2; exit 1; }

locate_crate() {
  # Prefer the immutable cargo cache copy; fall back to downloading the
  # pinned artifact. Either way the sha256 pin below is what matters.
  local f
  for f in "$HOME"/.cargo/registry/cache/*/iocraft-${CRATE_VERSION}.crate; do
    [ -e "$f" ] && { printf '%s' "$f"; return 0; }
  done
  command -v curl >/dev/null 2>&1 ||
    die "no cached iocraft-${CRATE_VERSION}.crate under ~/.cargo/registry/cache and no curl to download ${DOWNLOAD_URL}"
  local t
  t="$(mktemp)"
  curl -fsSL -o "$t" "$DOWNLOAD_URL" || { rm -f "$t"; die "download failed: ${DOWNLOAD_URL}"; }
  printf '%s' "$t"
}

verify_hash() {
  local crate="$1" actual
  read -r actual _ < <(sha256sum "$crate")
  [ "$actual" = "$CRATE_SHA256" ] ||
    die "iocraft-${CRATE_VERSION}.crate sha256 is ${actual}, the pin is ${CRATE_SHA256} — a different (or tampered) artifact; refusing to apply the patch to it"
}

verify_patch() {
  # The patch is an artifact too: a truncated or altered patch can still
  # 'apply' cleanly and leave a half-patched, unbuildable tree (verified:
  # dropping the new-file hunk applies with no error). Pin its sha256 so
  # ANY change to the patch — tamper or re-generation — is named here.
  # A legitimate re-generate updates this pin in the same commit.
  local actual
  [ -f "${ROOT}/${PATCH}" ] || die "missing ${PATCH} — the repo carries the patch; this checkout is broken"
  read -r actual _ < <(sha256sum "${ROOT}/${PATCH}")
  [ "$actual" = "$PATCH_SHA256" ] ||
    die "${PATCH} sha256 is ${actual}, the pin is ${PATCH_SHA256} — the patch was modified (or re-generated without updating the pin); refusing to apply it"
}

# The tree must be EXACTLY pristine@pin + patch. Re-materialize that
# ground truth in a temp dir and byte-compare; build dirs and cargo
# bookkeeping files are the only tolerated strays.
check_tree() {
  local crate="$1" work pristine diffout
  [ -d "$DEST" ] ||
    die "vendor/iocraft is missing — the tree is generated, not committed (plan 013-02). Run 'tools/apply-iocraft-patch.sh' once (README: one line under Install), then build."
  verify_patch
  work="$(mktemp -d)"
  tar -xzf "$crate" -C "$work"
  pristine="${work}/iocraft-${CRATE_VERSION}"
  ( cd "$pristine" && git apply -p1 "${ROOT}/${PATCH}" )
  diffout="${work}/tree-drift.diff"
  if diff -r --exclude=target --exclude=.cargo-ok --exclude=.gitignore \
      "$pristine" "$ROOT/$DEST" > "$diffout" 2>&1; then
    rm -rf "$work"
    return 0
  fi
  die "vendor/iocraft is STALE or manually modified — it differs from pristine iocraft-${CRATE_VERSION} (sha256 ${CRATE_SHA256}) + ${PATCH}. Drift diff left at ${diffout}; re-materialize with 'tools/apply-iocraft-patch.sh' (no args)."
}

apply_tree() {
  local crate work
  crate="$(locate_crate)"
  verify_hash "$crate"
  verify_patch
  work="$(mktemp -d)"
  tar -xzf "$crate" -C "$work"
  rm -rf "${ROOT}/${DEST}"
  mkdir -p "$(dirname "${ROOT}/${DEST}")"
  mv "${work}/iocraft-${CRATE_VERSION}" "${ROOT}/${DEST}"
  if ! ( cd "${ROOT}/${DEST}" && git apply --check -p1 "${ROOT}/${PATCH}" ) > "${work}/patch-err" 2>&1; then
    rm -rf "${ROOT}/${DEST}"
    die "the patch does not apply to pristine iocraft-${CRATE_VERSION} — a version mismatch or a stale patch. Re-generate patches/iocraft-${CRATE_VERSION}-use_cursor_position.patch against the new source (or retire it); do not build. Details: $(sed -n 1,8p "${work}/patch-err")"
  fi
  ( cd "${ROOT}/${DEST}" && git apply -p1 "${ROOT}/${PATCH}" )
  rm -rf "$work"
  # Belt-and-suspenders: the markers the patch exists to add must be
  # present after the apply (they will be — this is paranoia, not
  # expectation).
  grep -q "fn emit_cursor_position" "${ROOT}/${DEST}/src/backend/crossterm.rs" ||
    die "patch applied but its emit marker is absent — aborting"
  grep -q "pub use use_cursor_position" "${ROOT}/${DEST}/src/hooks/mod.rs" ||
    die "patch applied but the use_cursor_position re-export is absent — aborting"
  printf 'apply-iocraft-patch: vendor/iocraft materialized — pristine iocraft %s (sha256 %s) + %s (%s-file patch)\n' \
    "$CRATE_VERSION" "$CRATE_SHA256" "$PATCH" "$(grep -c '^+++' "${ROOT}/${PATCH}")"
}

case "${1:-apply}" in
  apply)
    apply_tree
    ;;
  check)
    crate="$(locate_crate)"
    verify_hash "$crate"
    check_tree "$crate"
    ;;
  *)
    printf 'usage: tools/apply-iocraft-patch.sh {apply|check}\n' >&2
    exit 2
    ;;
esac
