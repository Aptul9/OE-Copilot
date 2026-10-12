#!/bin/bash
# esegui.sh <scenario> <file di uscita>: lo script vero strumenti/sincronizza_telefono.sh con docker, sleep, date, python3 e install finti, su
# un telefono inventato (test/telefono-simulato.ts). Nessun telefono, nessun docker: solo i percorsi /opt/oe-copilot di una copia dello
# script sono sostituiti con una cartella di prova. La traccia (comandi al telefono, attese, battito, rilevatore) è quella che
# test/ciclo.test.ts si aspetta in test/golden/ciclo-<scenario>.txt. Serve bash con coreutils GNU e node 26 (Git Bash va bene).
set -u
QUI="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$QUI/../../../strumenti/sincronizza_telefono.sh"
NOME="$1"; USCITA="$2"
W="$QUI/run/$NOME"
if command -v cygpath >/dev/null 2>&1; then QUIW="$(cygpath -m "$QUI")"; WW="$(cygpath -m "$W")"; else QUIW="$QUI"; WW="$W"; fi
rm -rf "$W"; mkdir -p "$W/bin"
export SUP_W="$WW" SUP_SCENARIO="$NOME" MSYS_NO_PATHCONV=1
for t in docker date python3; do
    printf '#!/bin/sh\nexec node "%s/stub.mjs" %s "$@"\n' "$QUIW" "$t" > "$W/bin/$t"
done
# l'ultimo sleep del giro ferma lo script: il finto sleep esce con 99 e uccide il processo che lo ha lanciato
printf '#!/bin/sh\nnode "%s/stub.mjs" sleep "$@"\nrc=$?\n[ "$rc" = 99 ] && kill -TERM "$PPID"\nexit "$rc"\n' "$QUIW" > "$W/bin/sleep"
printf '#!/bin/sh\nexit 0\n' > "$W/bin/install"
chmod +x "$W/bin/"*
sed "s#/opt/oe-copilot#$W/opt/oe-copilot#g" "$SCRIPT" > "$W/sincronizza_telefono.sh"
node "$QUIW/inizia.mjs" || exit 3
eval "$(node "$QUIW/impostazioni.mjs")"
PATH="$W/bin:$PATH" timeout 300 bash "$W/sincronizza_telefono.sh" > "$W/script.out" 2> "$W/script.err"
cp "$W/traccia.txt" "$USCITA"
rm -rf "$W"
