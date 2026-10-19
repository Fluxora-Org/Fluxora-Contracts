#!/usr/bin/env bash
# Shared release plan. Keep the ordered list here so the real release and the
# dry-run cannot silently grow different artifact steps.

RELEASE_STEPS=(
  build_product_artifact
  verify_product_artifact
)

print_release_steps() {
  printf '%s\n' "${RELEASE_STEPS[@]}"
}
