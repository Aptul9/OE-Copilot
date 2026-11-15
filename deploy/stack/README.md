# One compose project on oe-vm

The OE Copilot calendar (`/opt/oe-copilot`), TeamsRelay (`/opt/teamsrelay`) and the remote MCP (`/opt/communication-mcp`) run as one compose project, `oe-stack`. Each module keeps its compose file, its folder, its `.env` and its GitHub workflow; `compose.yml` here includes the three.

## Pieces

| Path on oe-vm | From | Owner, mode |
|---|---|---|
| `/opt/oe-stack/compose.yml` | `compose.yml` | root, 644 |
| `/usr/local/sbin/oe-stack` | `oe-stack` | root, 755 |
| `/etc/sudoers.d/oe-stack` | below | root, 440 |
| `/opt/oe-stack/secrets/calendar.env` | written by `oe-stack calendar secrets` | root, 600 |

`oe-stack` runs `docker compose -f /opt/oe-stack/compose.yml` as root with an empty environment. Two reasons: compose reads every module's `.env`, readable only by its owner (`oecopilot`, `teamsrelay`), and a variable of the caller's shell would override the same variable in every `.env` (the modules share names such as `ADMIN_EMAIL`, `BETTER_AUTH_SECRET`, `VAPID_SUBJECT`).

The calendar's `API_KEY`, `ADMIN_EMAIL`, `ADMIN_PASSWORD`, `BETTER_AUTH_SECRET` and `TEAMS_BROKER_KEY` are not in its `.env`: its workflow sends them to `aggiorna_server.sh` on stdin, which hands them to `oe-stack calendar secrets`.

The deploy scripts (`calendar/server/strumenti/aggiorna_server.sh`, `teams-relay/deploy/remote-deploy.sh`, the deploy step of `.github/workflows/mcp.yml`) use the wrapper when `/usr/local/sbin/oe-stack` exists and act on their own services only (`oe-stack <stack> services`); without it they run the compose of their folder.

The names of networks, volumes and images are fixed in the module files (`oe-copilot_default`, `oe-copilot-teams`, `teamsrelay_default`, `teamsrelay_desktop`, `oe-copilot_caddy_dati`, `teamsrelay_caddy_data`...): the same as when each module was its own project, so certificates and data stayed where they were.

## Install or update the wrapper and the compose file

The deploy users cannot write these files: after a change to `compose.yml` or `oe-stack`, as root on oe-vm, from a checkout of the repository:

```bash
install -d -m 755 /opt/oe-stack
install -d -m 700 /opt/oe-stack/secrets
install -m 644 deploy/stack/compose.yml /opt/oe-stack/compose.yml
install -m 755 deploy/stack/oe-stack /usr/local/sbin/oe-stack
printf 'teamsrelay,oecopilot ALL=(root) NOPASSWD: /usr/local/sbin/oe-stack\n' > /etc/sudoers.d/oe-stack
chmod 440 /etc/sudoers.d/oe-stack && visudo -c
oe-stack all config -q
```

## Use

```bash
sudo oe-stack all ps                      # the containers of the three modules
sudo oe-stack teamsrelay logs -f webapp   # any compose command
sudo oe-stack calendar services           # agent gateway caddy-ui
```

## Back to three projects

The module files still work on their own (TeamsRelay without `docker-compose.oe.yml`, whose network the calendar creates). As root:

```bash
oe-stack all down                         # networks and volumes stay
rm /usr/local/sbin/oe-stack               # the deploy scripts go back to their folder's compose
cd /opt/oe-copilot && set -a && . /opt/oe-stack/secrets/calendar.env && set +a && docker compose up -d
cd /opt/teamsrelay && docker compose -f docker-compose.yml -f docker-compose.oe.yml up -d
cd /opt/communication-mcp && docker compose up -d
```
