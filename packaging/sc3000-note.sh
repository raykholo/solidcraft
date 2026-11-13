#!/usr/bin/env bash
# Adds a small fixed note to the SolidCraft 3000 page saying it is a parody, with a link to the real SolidCraft.
set -euo pipefail
page="${1:?usage: sc3000-note.sh <index.html>}"
note='<div id="sc3000-note" style="position:fixed;right:8px;bottom:8px;z-index:2147483647;max-width:360px;padding:6px 10px;border-radius:6px;background:rgba(20,22,28,.88);color:#e8eaf0;font:12px/1.4 system-ui,sans-serif">SolidCraft 3000 is a parody: there is no cloud and no account. The CAD underneath is the real <a href="https://github.com/tangency-labs/solidcraft" style="color:#8ab4ff">SolidCraft</a>, free and open source.</div>'
grep -q 'id="sc3000-note"' "$page" && exit 0
python3 - "$page" "$note" <<'PY'
import sys
p, note = sys.argv[1], sys.argv[2]
s = open(p, encoding="utf-8").read()
i = s.rfind("</body>")
if i < 0:
    sys.exit("no </body> in " + p)
open(p, "w", encoding="utf-8").write(s[:i] + note + "\n" + s[i:])
PY
