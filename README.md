# OE Copilot

OE Copilot unisce in un calendario solo i calendari di tutti gli account, aziendali e personali, con un colore per azienda. Li legge un agente sul server, da un telefono Android virtuale con Outlook e i servizi Google: niente registrazione su Azure, niente consenso dell'amministratore. Il calendario si apre nel browser, dietro HTTPS e una pagina di accesso. C'è anche una sezione Timesheet con i gestionali delle aziende.

- **[GUIDA.md](GUIDA.md)**: architettura, installazione, aggiornamento, configurazione e diagnosi del server (`server/`).
- **[app/README.md](app/README.md)**: il client per Windows e Android (`app/`), con i promemoria delle riunioni.

Funzioni facoltative: secondo fattore dell'accesso (GUIDA.md, 4.7), promemoria (4.9), condivisione del calendario con un altro OE Copilot (4.10).

## Personalizzazione

Nomi delle aziende, ordine dei calendari, sigle, loghi e schede del Timesheet stanno in `personalizzazione.json`, sul server in `ui-dati/` (GUIDA.md, 4.7), fuori dal repository. Senza il file ogni calendario prende il nome dal dominio della sua casella. Il gateway lo rilegge quando cambia. `personalizzazione.esempio.json` ha un esempio di ogni voce.

| Voce | A cosa serve |
|------|--------------|
| `nomi_per_indirizzo` | Nome per una casella precisa |
| `nomi_per_dominio` | Nome per tutte le caselle di un dominio |
| `domini_aziendali` | Un invito organizzato da questi domini prende il nome dell'azienda, anche nel calendario di un'altra casella |
| `caselle_di_colleghi` | Caselle di altre persone aperte in Outlook: i loro eventi restano loro |
| `ordine_calendari` | Ordine nella barra laterale |
| `azienda_per_cartella` | Cartelle calendario di un'azienda, riconosciute dal nome |
| `account_nascosti` | Caselle da non mostrare mai |
| `mostra_tutto_il_giorno` | `true` mostra gli eventi di un giorno intero delle caselle |
| `aziende_inoltrate` | Aziende le cui riunioni arrivano copiate in un'altra casella (sotto) |
| `interfaccia` | Sigle, loghi, calendari secondari, calendari senza "Apri in Teams", schede Timesheet (`schede_timesheet`: un clic apre `url`) |

Un logo è `"logos/nome.png"` (file in `ui-dati/logos/`) o un indirizzo, che scarica il server. Il logo scelto in *Gestisci calendari* vince sul file.

### Aziende inoltrate

Un flusso (per esempio Power Automate) può copiare le riunioni di un account aperto solo nel browser nel calendario di un'altra casella, con un prefisso nel titolo come `Copia: Riunione`. In `aziende_inoltrate` vanno la casella che le riceve (`via`), il prefisso (`prefisso`, espressione regolare) e, facoltativo, il `colore`. Le copie si mostrano come l'azienda d'origine, senza prefisso. Il pallino segue la casella che le ospita: un flusso fermo non si vede.

### Calendari Google

Col link iCal segreto, da *Gestisci calendari > Aggiungi un calendario da link*: vale come una password e resta sul server. Se l'amministratore l'ha disattivato, l'account si aggiunge nel telefono virtuale (GUIDA.md, 4.4).

## Prima di un push

- `python server/strumenti/controlla_dati_personali.py <personalizzazione.json>` fallisce se trova i dati della personalizzazione nei file del repository.
- La Action (`.github/workflows/server.yml`) prova e distribuisce solo `server/` e il workflow. `app/` si prova a mano (`app/README.md`).

## Se qualcosa non va

Log: `docker logs oe-copilot-<servizio>` sul server. Altri casi: GUIDA.md, sezione 7.

- **La pagina chiede di nuovo l'accesso**: il cookie dura 30 giorni, e un cambio di `ADMIN_PASSWORD` chiude le sessioni.
- **Secondo fattore perso**: `AUTH_2FA=off` in `.env` sul server e `gh workflow run Server --repo danfortu/OE-Copilot --ref main`. Basta la password (GUIDA.md, 4.7).
- **Un calendario nuovo non compare**: *Gestisci calendari > Cerca calendari nuovi*. Poi: in Outlook sul telefono va spuntato, con *Sincronizza calendari* acceso, e deve stare in `agent/fonti.json` (GUIDA.md, 4.2 e 4.4).
- **"Il server dei calendari non risponde"**: `docker logs oe-copilot-agent` e `docker logs oe-copilot-gateway`. Il suggerimento di un pallino giallo o rosso dice cosa è fermo.
