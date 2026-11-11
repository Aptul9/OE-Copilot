#!/bin/bash
# Prove delle immagini prima di un deploy (il repository non ha CI): Caddy, l'agente e il gateway veri, come girano in
# compose, su dati inventati. Sono i passi che faceva la CI di OE Copilot. Serve Docker; gira anche in Git Bash su Windows.
#   bash calendar/scripts/integrazione.sh      (lo lancia scripts/check.sh calendar --docker)
set -euo pipefail
# Git Bash: i percorsi negli argomenti di docker sono quelli del container e non diventano percorsi di Windows; il resto
# (curl -o /dev/null) resta convertito
docker() { MSYS_NO_PATHCONV=1 command docker "$@"; }
cal=$(cd "$(dirname "$0")/.." && { pwd -W 2>/dev/null || pwd; })
# una cartella temporanea che docker e node capiscono anche su Windows
cartella() { local d; d=$(mktemp -d); chmod 755 "$d"; if command -v cygpath >/dev/null; then cygpath -m "$d"; else echo "$d"; fi; }
lavoro=$(mktemp -d)
cd "$lavoro"
trap 'rm -rf "$lavoro"' EXIT

echo "== Caddy dell'interfaccia web"
docker build -q -t oe-copilot-caddy "$cal/server/caddy-ui"
docker run --rm -e UI_HOST=oe.example.com \
  oe-copilot-caddy caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile

echo "== Agente, immagine e risposte"
(
  # l'immagine parte davvero come in compose (sola lettura, utente senza privilegi, senza capacità) su un calendar.db inventato e
  # risponde con le risposte registrate in server/strumenti/golden/agente.json (le diede una volta l'agente Python, tolto dal repository).
  # L'immagine è quella in Rust; il database e il confronto vengono dagli strumenti di agente-ts
  docker build -q -t oe-copilot-agent "$cal/server/agente-rs"
  dati=$(cartella)
  node "$cal/server/agente-ts/test/costruisci-db.ts" "$dati"
  id=$(docker run -d --read-only --tmpfs /tmp --user 10001:10001 --cap-drop ALL --security-opt no-new-privileges:true -p 8181:8180 \
    -e API_KEY=chiave-di-prova -e BATTITO=/nessun-battito.json -e ACCESSI=/nessun-accesso.json -e TOKENS=/nessun-token.json \
    -v "$dati:/android-calendario:ro" -v "$dati/fonti.json:/app/fonti.json:ro" oe-copilot-agent)
  trap 'rc=$?; [ $rc = 0 ] || docker logs "$id"; docker rm -f "$id" >/dev/null' EXIT
  for _ in $(seq 1 30); do curl -s -o /dev/null localhost:8181/api/salute && break; sleep 1; done
  test "$(curl -s -o /dev/null -w '%{http_code}' localhost:8181/api/salute)" = 401
  # il controllo di salute del compose, dentro il container
  docker exec "$id" /agente --salute
  curl -fsS -H 'Authorization: Bearer chiave-di-prova' localhost:8181/api/salute | grep -q '"fonti": {"Alfa"'
  node "$cal/server/agente-ts/test/confronta-http.ts" http://localhost:8181
)

echo "== Gateway, accesso e cancello"
(
  # il gateway vero, dall'immagine: il cancello, l'accesso, il blocco per account, l'uscita, i metodi, i loghi e il secondo fattore (spento)
  docker build -q -t oe-copilot-gateway "$cal/server/web-ts"
  docker network rm prova >/dev/null 2>&1 || true
  docker network create prova >/dev/null
  pw=$(openssl rand -hex 16 | tr -d '\r')
  gw=$(docker run -d --network prova --network-alias gateway -p 3190:3000 -e GATEWAY_DB=/tmp/gw.db \
    -e OE_DATI=/tmp/dati -e AGENTE_URL=http://agent:8180 -e AGENTE_CHIAVE=prova \
    -e APP_URL=http://localhost:3190 -e BETTER_AUTH_SECRET=$(openssl rand -hex 32) \
    -e ADMIN_EMAIL=prova@example.com -e ADMIN_PASSWORD="$pw" oe-copilot-gateway)
  cd=$(docker run -d --network prova -p 8080:8080 -e UI_HOST=http://:8080 oe-copilot-caddy)
  # l'agente vero (la sua immagine) su un calendar.db inventato: il calendario condiviso legge gli eventi da lui, e senza risponde 503
  docker build -q -t oe-copilot-agent "$cal/server/agente-rs"
  dbd=$(cartella); node "$cal/server/agente-ts/test/costruisci-db.ts" "$dbd"
  ag=$(docker run -d --network prova --network-alias agent --read-only --tmpfs /tmp --user 10001:10001 --cap-drop ALL --security-opt no-new-privileges:true -e API_KEY=prova \
    -e BATTITO=/nessun-battito.json -e ACCESSI=/nessun-accesso.json -e TOKENS=/nessun-token.json -v "$dbd:/android-calendario:ro" -v "$dbd/fonti.json:/app/fonti.json:ro" oe-copilot-agent)
  trap 'rc=$?; [ $rc = 0 ] || { docker logs "$gw"; docker logs "$cd"; docker logs "$ag"; [ -z "${pk:-}" ] || docker logs "$pk"; }; docker rm -f "$gw" "$cd" "$ag" ${pk:-} >/dev/null; docker network rm prova >/dev/null' EXIT
  for _ in $(seq 1 30); do curl -fsS localhost:3190/salute >/dev/null 2>&1 && break; sleep 1; done
  for _ in $(seq 1 30); do curl -fsS localhost:8080/ -H 'Accept: text/html' -o /dev/null -w '%{http_code}' 2>/dev/null | grep -q 307 && break; sleep 1; done
  # senza AUTH_2FA il gateway parte in modalità off, e le tabelle del secondo fattore ci sono comunque (si aggiungono, non si tolgono)
  docker logs "$gw" 2>&1 | grep -q '(AUTH_2FA): off'
  docker exec -i "$gw" node - <<'JS'
const D = require('better-sqlite3');
const db = new D('/tmp/gw.db', { readonly: true });
const tabelle = db.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").pluck().all();
const colonne = db.prepare('PRAGMA table_info("user")').all().map((c) => c.name);
for (const t of ['twoFactor', 'passkey', 'login_failures', 'condivisioni', 'loghi']) if (!tabelle.includes(t)) throw new Error('manca la tabella ' + t);
if (!colonne.includes('twoFactorEnabled')) throw new Error('manca la colonna user.twoFactorEnabled');
JS
  G=http://localhost:3190
  codice() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
  json=(-H 'Content-Type: application/json' -H "Origin: $G")
  # senza sessione: la pagina rimanda a /login, l'API risponde 401, /login e /salute sono aperti
  test "$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' $G/)" = "307 $G/login?next=%2F"
  test "$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' $G/logos/personale.png)" = "302 $G/login?next=%2Flogos%2Fpersonale.png"
  test "$(codice -X POST "${json[@]}" -d '[]' $G/api/get_config)" = 401
  test "$(codice -X POST "${json[@]}" -d '["Alfa","https://example.org/logo.png"]' $G/api/set_logo)" = 401
  curl -fsS "$G/login?next=%2F" | grep -q 'Accedi a OE Copilot'
  # la disponibilità è pubblica: la pagina e i suoi dati si raggiungono senza sessione (qui il servizio non ha caselle:
  # nessun impegno, ma né 401 né reindirizzamento), ed è il calendario di sempre senza le parti che scrivono
  oggi=$(date -u +%F); fra7=$(date -u -d '+7 days' +%F)
  code=$(codice "$G/api/disponibilita?da=$oggi&a=$fra7"); [[ $code == 200 || $code == 502 ]]
  test "$(codice "$G/api/disponibilita?da=$oggi&a=$fra7&gruppo=nessuno")" = 404
  test "$(codice "$G/api/disponibilita?da=ieri&a=oggi")" = 400
  curl -fsS "$G/disponibilita" | grep -q '<title>Prenota un meeting'
  # la prenotazione è pubblica come la disponibilità: risponde senza sessione, e rifiuta quello che non è JSON o non rispetta le regole
  test "$(curl -s -o /dev/null -w '%{http_code}' -X POST -d x "$G/api/prenota")" = 415
  curl -s -X POST -H 'Content-Type: application/json' -d '{"inizio":"2026-10-31T10:00:00","fine":"2026-10-31T10:30:00","nome":"Prova","email":"prova@example.com"}' "$G/api/prenota" | grep -q 'lunedì al venerdì'
  test -z "$(curl -fsS "$G/disponibilita" | grep -o 'Nuovo evento')"
  # password sbagliata, poi giusta: cookie HttpOnly, e dopo l'accesso passa tutto
  test "$(codice -X POST "${json[@]}" -d '{"email":"prova@example.com","password":"sbagliata-sbagliata"}' $G/api/auth/sign-in/email)" = 401
  curl -fsS -c jar -D intest -o /dev/null -X POST "${json[@]}" -d "{\"email\":\"prova@example.com\",\"password\":\"$pw\"}" $G/api/auth/sign-in/email
  grep -qi 'set-cookie: .*HttpOnly' intest
  # la pagina principale è l'interfaccia in TypeScript; la pagina vecchia (index.html) non c'è più
  curl -fsS -b jar $G/ | grep -q 'Cerca tra le riunioni'
  test "$(codice -b jar $G/index.html)" = 404
  # il secondo fattore è spento: l'accesso non ha il passo del codice, e le sue strade e la sua pagina non ci sono (404)
  test -z "$(curl -fsS -X POST "${json[@]}" -d "{\"email\":\"prova@example.com\",\"password\":\"$pw\"}" $G/api/auth/sign-in/email | grep -o twoFactorRedirect)"
  test "$(codice -X POST "${json[@]}" -d '{"code":"123456"}' $G/api/auth/two-factor/verify-totp)" = 404
  test "$(codice -b jar -X POST "${json[@]}" -d '{"password":"x"}' $G/api/auth/two-factor/enable)" = 404
  test "$(codice $G/api/auth/passkey/generate-authenticate-options)" = 404
  test "$(codice -b jar $G/api/auth/passkey/list-user-passkeys)" = 404
  test "$(codice -b jar $G/sicurezza)" = 404
  # i loghi li serve il gateway: quello dell'immagine (logos-predefiniti), uno messo nella cartella dei dati
  # subito, senza riavviare niente, e mai un file fuori dalle due cartelle (server.js sta accanto a logos-predefiniti)
  test "$(codice -b jar $G/logos/personale.png)" = 200
  test "$(curl -s -b jar -o /dev/null -w '%{http_code} %{content_type}' $G/logos/personale.png)" = "200 image/png"
  test "$(codice -b jar -I $G/logos/personale.png)" = 200
  test "$(codice -b jar $G/logos/nessuno.png)" = 404
  docker exec "$gw" sh -c 'mkdir -p /tmp/dati/logos && cp /app/logos-predefiniti/personale.png /tmp/dati/logos/prova.svg'
  test "$(curl -s -b jar -o /dev/null -w '%{http_code} %{content_type}' $G/logos/prova.svg)" = "200 image/svg+xml"
  test "$(codice -b jar "$G/logos/..%2fserver.js")" = 404
  test "$(codice -b jar "$G/logos/%2e%2e%2f%2e%2e%2f%2e%2e%2fetc%2fpasswd")" = 404
  test "$(codice -b jar "$G/logos/%2fapp%2fserver.js")" = 404
  curl -fsS -b jar -X POST "${json[@]}" -d '[]' $G/api/get_personalizzazione | grep -q '"interfaccia"'
  # ogni logo porta la sua politica (nessuno script, nessuna risorsa), anche aperto da solo in una scheda
  curl -s -b jar -D intestl -o /dev/null $G/logos/personale.png
  grep -qi "^content-security-policy: default-src 'none'; style-src 'unsafe-inline'; sandbox" intestl
  grep -qi '^x-content-type-options: nosniff' intestl
  # i loghi da indirizzo: il gateway scarica solo siti pubblici, quindi la rete interna, i metadati del cloud e gli schemi che non
  # sono http si rifiutano prima di collegarsi (nessun traffico in uscita); la tabella dei loghi non è nella configurazione
  for url in http://127.0.0.1/x.png http://localhost/x.png http://169.254.169.254/latest/meta-data/ 'http://[::1]/x.png' 'http://[::ffff:0:127.0.0.1]/x.png' http://10.0.0.5/x.png file:///etc/passwd; do
    for metodo in set_logo preview_logo; do
      if [ "$metodo" = set_logo ]; then corpo="[\"Alfa\",\"$url\"]"; else corpo="[\"$url\"]"; fi
      curl -fsS -b jar -X POST "${json[@]}" -d "$corpo" "$G/api/$metodo" | grep -q '"error":"Indirizzo non ammesso"'
    done
  done
  # un file che prima fermava il controllo delle immagini (22 commenti vuoti e una x) si rifiuta subito: il gateway non resta fermo
  ostile=$( { for i in $(seq 22); do printf '<!---->'; done; printf x; } | base64 -w0 )
  curl -fsS --max-time 5 -b jar -X POST "${json[@]}" -d "[\"Alfa\",\"$ostile\"]" $G/api/upload_logo | grep -q "\"error\":\"Non è un'immagine\""
  curl -fsS -b jar -X POST "${json[@]}" -d '[]' $G/api/get_loghi | grep -q '"loghi":{}'
  test -z "$(curl -fsS -b jar -X POST "${json[@]}" -d '[]' $G/api/get_config | grep -o logo)"
  # i calendari da link passano dalla stessa guardia: la rete interna, i metadati del cloud, i nomi dei servizi e uno schema che non è http
  # si rifiutano prima di collegarsi (nessun traffico in uscita), e nella configurazione non resta niente
  for url in http://127.0.0.1/c.ics http://localhost/c.ics http://169.254.169.254/latest/meta-data/ 'http://[::1]/c.ics' 'http://[::ffff:0:127.0.0.1]/c.ics' http://10.0.0.5/c.ics http://agent:8180/api/ics http://gateway/c.ics; do
    curl -fsS --max-time 5 -b jar -X POST "${json[@]}" -d "[\"Prova\",\"$url\"]" $G/api/add_ics_calendar | grep -q '"error":"Il link non risponde: Indirizzo non ammesso"'
  done
  curl -fsS --max-time 5 -b jar -X POST "${json[@]}" -d '["Prova","file:///etc/passwd"]' $G/api/add_ics_calendar | grep -q '"error":"Il link deve iniziare con https:// (o webcal://)."'
  curl -fsS -b jar -X POST "${json[@]}" -d '[]' $G/api/get_config | grep -q '"ics_calendars":\[\]'
  # i metodi della lettura li serve il gateway da sé (qui senza agente: nessuna casella, ma la risposta c'è, in JSON e senza cache)
  curl -fsS -b jar -D intest2 -X POST "${json[@]}" -d '[]' $G/api/get_config | grep -q '"theme"'
  grep -qi '^content-type: application/json' intest2
  curl -fsS -b jar -X POST "${json[@]}" -d '[]' $G/api/get_calendars | grep -q '"calendars"'
  test "$(codice -b jar -X POST "${json[@]}" -d '{"a":1}' $G/api/get_config)" = 400
  test "$(curl -s -b jar -X POST "${json[@]}" -d '["boh"]' $G/api/get_disponibilita | grep -c 'Intervallo non valido')" = 1
  # le scritture le serve il gateway da sé: lo stato sta nel suo database e config.json con gli eventi personali si riscrivono
  # nella cartella dei dati (qui /tmp/dati, che il gateway crea)
  curl -fsS -b jar -X POST "${json[@]}" -d '["remoto::prova", false]' $G/api/set_calendar_enabled | grep -q '"ok":true'
  curl -fsS -b jar -X POST "${json[@]}" -d '[]' $G/api/get_config | grep -q '"disabled_calendars":\["remoto::prova"\]'
  curl -fsS -b jar -X POST "${json[@]}" -d '[{"title":"Prova","start":"2026-10-30T20:00:00"}]' $G/api/add_local_event | grep -q '"ok":true'
  curl -fsS -b jar -X POST "${json[@]}" -d '[{"title":"","start":"x"}]' $G/api/add_local_event | grep -q 'Il titolo è obbligatorio'
  curl -fsS -b jar -X POST "${json[@]}" -d '[{"theme":"dark"}]' $G/api/save_config | grep -q '"ok":true'
  docker exec "$gw" grep -q '"remoto::prova"' /tmp/dati/config.json
  docker exec "$gw" grep -q '"title": "Prova"' /tmp/dati/local_events.json
  test "$(codice -b jar -X POST "${json[@]}" -d '[]' $G/api/win_close)" = 404
  test "$(codice -b jar -X POST -d '[]' $G/api/get_config)" = 415
  test "$(codice -b jar -X POST "${json[@]}" -H 'Sec-Fetch-Site: cross-site' -d '[]' $G/api/get_config)" = 403
  # il sito dietro Caddy: senza sessione rimanda a /login, con la sessione serve la pagina
  C=http://localhost:8080
  test "$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' $C/)" = "307 $C/login?next=%2F"
  curl -fsS -b jar $C/ | grep -q 'Cerca tra le riunioni'
  # calendario condiviso (README.md, 4.10): un indirizzo che non vale è 404 come un percorso che non c'è; con la sessione si crea un link, e il calendario si legge
  # senza sessione (text/calendar, rilettura ogni 30 minuti) e nel livello occupato non ha niente di quello che chi lo condivide ha scritto
  test "$(codice $G/condividi/prova.ics)" = 404
  test "$(codice "$G/condividi/$(openssl rand -hex 22 | cut -c1-43).ics")" = 404
  dopo=$(date -u -d '+3 days' +%F)
  curl -fsS -b jar -X POST "${json[@]}" -d "[{\"title\":\"Titolo riservato\",\"start\":\"${dopo}T10:00:00\",\"end\":\"${dopo}T11:00:00\",\"location\":\"Luogo riservato\",\"notes\":\"Note riservate\"}]" $G/api/add_local_event | grep -q '"ok":true'
  # l'agente vero ha una fonte che non si legge (assente@alfa.example: nessun calendario nel database), quindi l'azienda Alfa è in errore. Un link a tutti i
  # calendari la pubblica e senza i suoi eventi il calendario sarebbe incompleto (per chi lo importa: libero): risponde 503 con Retry-After e non pubblica
  # niente. Un link a un gruppo senza quell'azienda (Beta e gli eventi personali) risponde, ed è quello su cui valgono le prove qui sotto
  curl -fsS -b jar -X POST "${json[@]}" -d '[{"gruppi_calendari":{"prova":["Beta","Eventi personali"]}}]' $G/api/save_config | grep -q '"ok":true'
  tutti=$(curl -fsS -b jar -X POST "${json[@]}" -d '[{"nome":"Tutti"}]' $G/api/add_condivisione | jq -r .condivisione.url | tr -d '\r')
  [[ $tutti == $G/condividi/*.ics ]]
  curl -s -D tutti.h -o tutti.ics "$tutti"
  head -n 1 tutti.h | grep -q ' 503 '
  grep -qi '^retry-after: 120' tutti.h
  test -z "$(grep BEGIN:VCALENDAR tutti.ics)"
  cond=$(curl -fsS -b jar -X POST "${json[@]}" -d '[{"nome":"Prova","nome_calendario":"Calendario di prova","ambito":"gruppo","gruppo":"prova"}]' $G/api/add_condivisione)
  link=$(jq -r .condivisione.url <<<"$cond" | tr -d '\r'); lid=$(jq -r .condivisione.id <<<"$cond" | tr -d '\r')
  [[ $link == $G/condividi/*.ics ]]
  curl -fsS -D feed.h -o feed.ics "$link"
  grep -qi '^content-type: text/calendar; charset=utf-8' feed.h
  grep -q '^BEGIN:VCALENDAR' feed.ics
  grep -q '^X-PUBLISHED-TTL:PT30M' feed.ics
  grep -q '^X-WR-CALNAME:Calendario di prova' feed.ics
  grep -q '^SUMMARY:Occupato' feed.ics
  test -z "$(grep -E 'Titolo riservato|Luogo riservato|Note riservate' feed.ics)"
  test "$(codice -I "$link")" = 200
  etag=$(grep -i '^etag:' feed.h | cut -d' ' -f2 | tr -d '\r')
  test "$(codice -H "If-None-Match: $etag" "$link")" = 304
  # con i titoli escono titolo e luogo, mai le note; revocato, l'indirizzo dà 404 subito
  curl -fsS -b jar -X POST "${json[@]}" -d "[\"$lid\",{\"livello\":\"titoli\"}]" $G/api/update_condivisione | grep -q '"ok":true'
  curl -fsS "$link" >titoli.ics
  grep -q '^SUMMARY:Titolo riservato' titoli.ics
  grep -q '^LOCATION:Luogo riservato' titoli.ics
  test -z "$(grep 'Note riservate' titoli.ics)"
  curl -fsS -b jar -X POST "${json[@]}" -d "[\"$lid\"]" $G/api/revoke_condivisione | grep -q '"ok":true'
  test "$(codice "$link")" = 404
  # uscita: la sessione non vale più
  curl -fsS -b jar -c jar -o /dev/null -X POST "${json[@]}" -d '{}' $G/api/auth/sign-out
  test "$(codice -b jar -X POST "${json[@]}" -d '[]' $G/api/get_config)" = 401
  test "$(curl -s -b jar -o /dev/null -w '%{http_code}' $C/)" = 307
  # cinque errori bloccano l'account, anche per la password giusta
  for i in 1 2 3 4 5; do codice -X POST "${json[@]}" -d '{"email":"prova@example.com","password":"sbagliata-sbagliata"}' $G/api/auth/sign-in/email >/dev/null; done
  test "$(codice -X POST "${json[@]}" -d "{\"email\":\"prova@example.com\",\"password\":\"$pw\"}" $G/api/auth/sign-in/email)" = 429
  # un valore di AUTH_2FA che non è off, totp o passkey ferma l'avvio, col motivo nel log
  if out=$(MSYS_NO_PATHCONV=1 timeout 90 docker run --rm -e AUTH_2FA=pippo -e GATEWAY_DB=/tmp/x.db -e OE_DATI=/tmp/x -e BETTER_AUTH_SECRET=$(openssl rand -hex 32) \
    -e ADMIN_EMAIL=prova@example.com -e ADMIN_PASSWORD="$pw" oe-copilot-gateway 2>&1); then echo "AUTH_2FA=pippo non ha fermato l'avvio"; exit 1; fi
  grep -q 'AUTH_2FA non valido' <<<"$out"
  # con AUTH_2FA=passkey il gateway parte dall'immagine (il plugin è nel pacchetto), le strade delle passkey ci sono e il sito è quello di APP_URL
  pk=$(docker run -d -p 3191:3000 -e AUTH_2FA=passkey -e GATEWAY_DB=/tmp/gw.db -e OE_DATI=/tmp/dati -e APP_URL=http://localhost:3191 \
    -e BETTER_AUTH_SECRET=$(openssl rand -hex 32) -e ADMIN_EMAIL=prova@example.com -e ADMIN_PASSWORD="$pw" oe-copilot-gateway)
  for _ in $(seq 1 30); do curl -fsS localhost:3191/salute >/dev/null 2>&1 && break; sleep 1; done
  docker logs "$pk" 2>&1 | grep -q '(AUTH_2FA): passkey'
  curl -fsS localhost:3191/api/auth/passkey/generate-authenticate-options | grep -q '"rpId":"localhost"'
  test "$(codice -X POST -H 'Content-Type: application/json' -H 'Origin: http://localhost:3191' -d '{"password":"x"}' localhost:3191/api/auth/two-factor/enable)" = 401
  test "$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' localhost:3191/sicurezza)" = "307 http://localhost:3191/login?next=%2Fsicurezza"
  # Caddy con il gateway fermo (502): l'errore di inoltro scrive la richiesta, e il testo segreto di un link di condivisione non deve starci, comunque sia
  # scritto l'indirizzo (// davanti, segmenti col punto, maiuscole, indirizzo intero, ovunque) né in Referer
  docker stop "$gw" >/dev/null
  tok=$(openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\r\n')
  for p in "/condividi/$tok.ics" "//condividi/$tok.ics" "/x/../condividi/$tok.ics" "/CONDIVIDI/$tok.ics" "/condividi/./$tok.ics" "/api/x?t=$tok"; do
    test "$(codice --path-as-is "$C$p")" = 502
  done
  test "$(codice --request-target "$C/condividi/$tok.ics" "$C/")" = 502
  test "$(codice -H "Referer: $C/condividi/$tok.ics" "$C/")" = 502
  sleep 1
  docker logs "$cd" >caddy.log 2>&1
  grep -q '/condividi/REDATTO' caddy.log
  test -z "$(grep -F -- "$tok" caddy.log)"
)
echo "== immagini: tutto a posto"
