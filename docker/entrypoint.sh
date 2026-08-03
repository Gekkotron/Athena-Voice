#!/bin/sh
set -eu

# Bundled-skill seeding/refresh for the /data volume. Paths are
# env-overridable so the logic is testable outside the image.
BUNDLED_DIR="${ATHENA_BUNDLED_DIR:-/app/skills}"
SKILLS_DIR="${ATHENA_SKILLS_DIR:-/data/skills}"
mkdir -p "$SKILLS_DIR"

# Rules, per bundled skill <name>.wasm:
#   - absent from the volume                  -> copy (first boot, or a new
#                                                bundled skill in this image)
#   - present and identical to what a         -> replace with this image's
#     previous boot seeded (see manifest)        version (update rides along)
#   - present but changed since seeding       -> leave untouched (the user
#     (user replaced/uploaded it)                manages this file now)
#
# The manifest records "<file> <sha256-as-seeded>" so user-managed state
# always wins over image updates.
MANIFEST="$SKILLS_DIR/.seeded-manifest"
touch "$MANIFEST"

for src in "$BUNDLED_DIR"/*.wasm; do
    [ -e "$src" ] || continue
    f=$(basename "$src")
    dst="$SKILLS_DIR/$f"
    src_sum=$(sha256sum "$src" | cut -d' ' -f1)
    if [ ! -f "$dst" ]; then
        cp "$src" "$dst"
        echo "seeded skill $f" >&2
    else
        dst_sum=$(sha256sum "$dst" | cut -d' ' -f1)
        seeded_sum=$(grep "^$f " "$MANIFEST" | tail -1 | cut -d' ' -f2)
        if [ "$dst_sum" = "$seeded_sum" ] && [ "$dst_sum" != "$src_sum" ]; then
            cp "$src" "$dst"
            echo "refreshed bundled skill $f (image update)" >&2
        fi
    fi
    grep -v "^$f " "$MANIFEST" > "$MANIFEST.tmp" || true
    printf '%s %s\n' "$f" "$src_sum" >> "$MANIFEST.tmp"
    mv "$MANIFEST.tmp" "$MANIFEST"
done

exec /app/athena-voice "$@"
