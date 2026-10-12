#!/bin/bash
# tutti.sh <cartella di uscita>: esegui.sh per ogni scenario, ciclo-<nome>.txt in <cartella di uscita>. Per rigenerare i golden:
#   bash server/supervisore/test/script-finto/tutti.sh server/supervisore/test/golden
QUI="$(cd "$(dirname "$0")" && pwd)"
if command -v cygpath >/dev/null 2>&1; then QUIW="$(cygpath -m "$QUI")"; else QUIW="$QUI"; fi
OUT="$1"; mkdir -p "$OUT"
for nome in $(node "$QUIW/elenco.mjs"); do
    echo "== $nome"
    bash "$QUI/esegui.sh" "$nome" "$OUT/ciclo-$nome.txt"
done
