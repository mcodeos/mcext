#!/bin/sh
# The repository-level release command.
#
# One command per repository: read the release contract, make the
# artifact, attach the manifest, pass the batch gate, carry it to the
# channel.  The scripts in this repository are the parts of that command,
# not the command itself.
#
# The workspace root is an environment fact, never a path written here:
#
#     MCC_WORKSPACE_ROOT=/path/to/workspace scripts/release.sh
#
set -eu

: "${MCC_WORKSPACE_ROOT:?the workspace root must be given: MCC_WORKSPACE_ROOT}"

# The signing key is an environment fact: the release is signed when the
# caller names a key, and the channel refuses an unsigned manifest.
if [ -n "${MCC_RELEASE_KEY:-}" ]; then
  exec mcgen-release package --package mcext --workspace "$MCC_WORKSPACE_ROOT" \
    --sign "$MCC_RELEASE_KEY"
fi

exec mcgen-release package --package mcext --workspace "$MCC_WORKSPACE_ROOT"
