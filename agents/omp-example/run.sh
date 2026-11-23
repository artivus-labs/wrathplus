#!/bin/sh
# Run the OMP example agent against a client whose control port is open.
#   WRATHPLUS_AGENT_URL, WRATHPLUS_AGENT_TOKEN   the client's control port and its token
#   ANTHROPIC_API_KEY                            the planner's key (or an `omp` login, see README)
#   WRATHPLUS_OMP_STATE                          this agent's own folder (default ~/.local/share/wrathplus-omp-agent)
# Everything else is optional; see README.md. OMP needs Bun 1.4.4+: this uses the example's own
# pinned Bun (node_modules/.bin), so the system Bun may be older.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
if [ ! -x "$here/node_modules/.bin/bun" ]; then
  echo "run 'bun install' in $here first (and in agents/mcp)" >&2
  exit 1
fi
if [ ! -d "$here/../mcp/node_modules" ]; then
  echo "run 'bun install' in $here/../mcp first" >&2
  exit 1
fi
export PATH="$here/node_modules/.bin:$PATH"
export PI_AUTO_QA=0 OTEL_SDK_DISABLED=true PI_NO_TITLE=1
exec bun run "$here/src/player.ts"
