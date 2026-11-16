# Call lab

Tools for live tests of the Rust calling worker (`teamsrelay-calling`) against real Teams clients: throwaway containers on oe-vm in docker network `calltest`, with their UDP captured on its bridge, and new Teams desktop on the laptop, driven through UI Automation. Nothing here goes into an image.

## Running

- oe-vm scripts are piped over ssh: `Get-Content -Raw <script> | ssh oe-vm "tr -d '\r' | VAR=value bash -s"`.
- Their helpers are copied to `/tmp` on oe-vm first: `scp pcap.py stuntrace.py firstrtp.py answered.py cbs.py harness.js calls-flag.js oe-vm:/tmp/`.
- The worker under test is the static build of `build.sh`, copied as `/tmp/teamsrelay-calling-dbg`; `CALLING_DEBUG=1` makes it log callbacks, SDP, candidates and ICE states.
- Token copies go to `/var/tmp/calltest` (root, 700). `cleanup.sh` removes them, the network `calltest` and the `/tmp` helpers, and prints the calls flag of every slot.

## Scripts

| Script | Runs on | Does |
|---|---|---|
| `build.sh` | WSL | static musl `teamsrelay-calling` from `SRC` (default the main checkout), build cache `~/trcalls-dbg*` |
| `test.sh` | WSL | cargo test (`FILTER`, default `calling`, empty for all), clippy, fmt |
| `call.sh` | oe-vm | one outgoing call in echo mode from slot `CALLER` (1) to oid `PEER` (Vection; `none` for the test-call service) for `SECONDS_TALK` s; every `CALLING_*` variable goes into the container; capture `cap-*.pcap`, log `native-*.log` |
| `run-out.ps1` | laptop | `-Runs N` outgoing calls through `call.sh`, each accepted by `uia-accept.ps1`; first RTP after Accept per run |
| `incoming.sh` | oe-vm | slot `SLOT` (6) answered by the worker in serve mode through `harness.js`, its live worker switched off meanwhile and back on after; the laptop calls with `msteams:/l/call/0/0?users=<upn>` and `uia-press.ps1` |
| `prodcall.sh` | oe-vm | a real call through the app: `call` command queued in `data/<SLOT>/messages.db` for chat `CHAT`, `hangup` after `TALK` s, UDP of teams-browsers captured |
| `postdeploy.sh` | oe-vm | deployed sha, worker check, running workers, recent errors, slot health, echo call with the deployed binary (`ECHO_SLOT`, 2) |
| `nodeprobe.sh` | oe-vm | the same call through the Node worker (`calling-live-host.mjs`, Teams web calling stack), as reference |
| `cleanup.sh` | oe-vm | test state removed, calls flags printed |
| `uia-accept.ps1` | laptop | presses Accept in the next new Teams window after `-DelayMs`, prints the press in unix ms |
| `uia-click.ps1` | laptop | the same with a real mouse click |
| `uia-press.ps1` | laptop | one button in one named dialog ("Avvia chiamata" in "Avviare la chiamata?") |
| `uia-dump.ps1` | laptop | buttons of the Teams windows with their ancestors |
| `teams-keys.ps1` | laptop | a key chord to the Teams main window; the default Ctrl+Alt+Shift+1 writes the support zip `Downloads\PROD-WebLogs-*.zip`, web log of each account in `User (<oid>)\diagnostics-logs.txt` |
| `clockoff.ps1` | laptop | offset of the oe-vm clock against the laptop |
| `pw-hook.js` | Playwriter | a Teams web tab with RTCPeerConnection wrapped and the calling HTTP and Trouter traffic recorded |

Analysis:

| Script | Reads | Gives |
|---|---|---|
| `pcap.py` | capture | DTLS records and RTP per direction; `ssrc` for SSRC and payload types |
| `stuntrace.py` | capture | every STUN message and DTLS record in time order, ChannelData mapped to its peer |
| `stunscan.py` | capture | ICE checks per direction and attribute set |
| `answered.py` | capture | inbound checks and how many got an answer |
| `firstrtp.py` | capture | first inbound RTP after DTLS and its delay from an Accept press |
| `hello.py` | capture | the first ClientHello decoded |
| `dtlscert.py` | capture | DTLS handshake details, peer certificate |
| `turn.py` | discovery | a TURN Allocate against the Teams relay |
| `cbs.py`, `cb.py` | debug log | callbacks of an echo-mode and of a serve-mode log |
| `lastlog.sh` | debug log | the latest echo-mode log without callback bodies |
| `gtdecode.py`, `gtcreate.py`, `gtcb.py` | `pw-hook.js` trace | traffic in order, the call-create body, one callback |
| `receipt.py` | webrtc-internals dump | peer connections and their last stats |
| `scrub.py` | trace | tokens removed in place |
| `who.js`, `slots.js`, `ident.js`, `chats.js`, `activecalls.js`, `calls-flag.js` | slot data in teams-browsers | token claims, flags, identity, chats, call state, calls flag set |

## Reference

- `teams-web-call-create.txt`: the call create of Teams web 2026.40 in Chrome (Vection to notarify, 2026-11-15), links stripped: body, offer SDP, headers.

## Traps

- A call to an account rings every device it is signed in on, and each sends its own mediaAnswer.
- The laptop and oe-vm clocks agree within about 50 ms (`clockoff.ps1`), so a press on the laptop compares with capture times on oe-vm.
