#!/usr/bin/env bash
# Fetch the pinned CRS checkout used by crs-diff-gate.sh.
#
# The CRS tree is not vendored in this repository. CI and local runs obtain it
# through this script so the revision is fixed by repository + tag + commit and
# verified before the gate runs. The commit SHA is a content address for the
# whole tree: a force-moved tag, an unreachable revision or a corrupted checkout
# fails closed instead of silently measuring a different corpus.
#
# Usage: crs-fetch.sh <destination-directory>
# The destination must be absent or empty; the script never reuses a stale tree.
set -euo pipefail

CRS_REPOSITORY=${CRS_REPOSITORY:-https://github.com/coreruleset/coreruleset.git}
CRS_REF=${CRS_REF:-v4.25.0}
CRS_COMMIT=${CRS_COMMIT:-aabf675fcfc5e489b424b844e6c9f1b39802df69}

die() {
    echo "crs-fetch: $*" >&2
    exit 2
}

DEST=${1:-}
if [[ -z "$DEST" ]]; then
    die "usage: crs-fetch.sh <destination-directory>"
fi
command -v git >/dev/null 2>&1 || die "git is required"
if [[ -e "$DEST" ]]; then
    if [[ ! -d "$DEST" ]]; then
        die "destination is not a directory: $DEST"
    fi
    if [[ -n "$(ls -A "$DEST" 2>/dev/null)" ]]; then
        die "destination is not empty: $DEST (pass a clean directory)"
    fi
fi

mkdir -p "$DEST"
git init --quiet "$DEST"
git -C "$DEST" remote add origin "$CRS_REPOSITORY"
if ! git -C "$DEST" fetch --quiet --depth 1 origin "refs/tags/$CRS_REF"; then
    die "could not fetch tag $CRS_REF from $CRS_REPOSITORY"
fi
git -C "$DEST" -c advice.detachedHead=false checkout --quiet FETCH_HEAD

ACTUAL_COMMIT=$(git -C "$DEST" rev-parse HEAD)
if [[ "$ACTUAL_COMMIT" != "$CRS_COMMIT" ]]; then
    die "commit mismatch for $CRS_REF: expected $CRS_COMMIT, got $ACTUAL_COMMIT"
fi
if [[ ! -f "$DEST/crs-setup.conf.example" || ! -d "$DEST/rules" ]]; then
    die "not a CRS checkout after fetch: $DEST"
fi

echo "crs_repository=$CRS_REPOSITORY"
echo "crs_ref=$CRS_REF"
echo "crs_commit=$ACTUAL_COMMIT"
echo "crs_dir=$DEST"
