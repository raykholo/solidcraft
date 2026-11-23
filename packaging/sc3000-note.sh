#!/usr/bin/env bash
# Adds a small fixed note to the SolidCraft 3000 page saying it is a parody, with a link to the real SolidCraft.
# English until the app starts; the app then puts the text either side of the link in the person's language
# (sc3000.web.note_before / note_after, every shipped locale).
set -euo pipefail
page="${1:?usage: sc3000-note.sh <index.html>}"
note='<div id="sc3000-note" style="position:fixed;left:8px;bottom:calc(8px + var(--sc3000-strip, 0px));z-index:2147483647;max-width:360px;padding:6px 10px;border-radius:6px;background:rgba(20,22,28,.88);color:#e8eaf0;font:12px/1.4 system-ui,sans-serif" dir="auto"><span id="sc3000-note-before">SolidCraft 3000 is a parody: there is no cloud and no account. The CAD underneath is the real </span><a href="https://github.com/tangency-labs/solidcraft" style="color:#8ab4ff">SolidCraft</a><span id="sc3000-note-after">, free and open source.</span></div>'
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
