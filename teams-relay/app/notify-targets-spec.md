# Notification targets per account

## Goal

Each Teams account sends its notifications to the devices its owner picks: the phones (Mobile: the Android app, and the browsers of phones and tablets), the browsers of computers (Desktop), both, or none. One control per account in Settings, in the row of the account.

## Decisions (owner, 2026-11-10)

| Topic | Decision |
|---|---|
| Control | One button per account row, opening a menu with two checkboxes: Mobile and Desktop. None ticked means off |
| Default | Both ticked: every existing account keeps today's behavior |
| Mobile | The phones of the Android app (Firebase, endpoint `fcm:`), and Web Push from a phone or tablet browser: an iPhone or iPad web app on the Home Screen, Chrome on Android |
| Desktop | Web Push from every other browser |
| Device kind | Decided once, when the browser subscribes: the client works it out from the browser and sends it with the subscription, the server stores it with it. Subscriptions made before this change count as Desktop until their device taps Enable notifications again |
| Off | No push for messages, calls, missed calls and what a check finds. The history (`messages` table, shown by `/api/feed`) and the "Last message" of the Status panel are still written |
| ntfy | Not used by the owner. Left as it is: still sent when configured, and not sent when the account is off |
| Account health | `alert()` stays unfiltered: signed out, back again, relay browser down or back, recheck result, self-check problem |
| Scope | Per account. No select-all |
| Menu behavior | Stays open while ticking. Each tick saves at once, optimistically, and rolls back on error |
| Where | The Teams accounts card in `Settings.tsx`, which stays inside the "Advanced Teams settings" fold. The summary becomes "Teams accounts: notifications, run mode and relay tokens" |
| Relation to `mcp-only-accounts-spec.md` | That spec is not implemented. If it lands, an MCP-only account stays silent whatever these flags say, and it moves the same two `alert()` callers this spec moves |

## Data and lifecycle

- New columns `teams_accounts.notify_mobile INTEGER NOT NULL DEFAULT 1` and `notify_desktop INTEGER NOT NULL DEFAULT 1`, added by the migration loop in `src/lib/appdb.ts` like `calls`, and in the `Slot` type and `SLOT_COLUMNS`.
- `setNotify(db, slot, {mobile?, desktop?})` in `src/lib/appdb.ts` updates only the columns given, so two quick ticks never overwrite each other.
- `PATCH /api/accounts/[slot]` accepts `{notifyMobile?, notifyDesktop?}`. Handled before the shared-account branch, as `calls` is. A value that is not a boolean: 400. The slot must belong to the caller (`requireAccount`).
- `AccountSummary` (`src/lib/accounts.ts`) carries `notifyMobile` and `notifyDesktop`, optional like `calls`: absent, as from a server of an earlier release, reads as on.
- Relay accounts (on another computer) and accounts on a shared Microsoft account have the control too. Both are notified through the server.
- A Web Push subscription is stored as `{endpoint, keys, kind}` in `push_subscriptions.sub`, `kind` being `"mobile"` or `"desktop"`. `subscriptionOf` (`src/shared/push-subscription.ts`, shared with the local relay API) reads an optional `device` from the body: `"mobile"` is kept, anything else or nothing is stored as `"desktop"`. `web-push` reads only `endpoint` and `keys`, so the extra field changes nothing on the wire.
- New `deviceKindOf(userAgent, touchPoints)` in the same file: `"mobile"` for `/Android|iPhone|iPad|iPod|Mobile/i`, and for a `Macintosh` user agent with more than one touch point (iPadOS reports itself as a Mac), else `"desktop"`. `enablePush` (`src/lib/push.ts`) posts `{...sub.toJSON(), device: deviceKindOf(navigator.userAgent, navigator.maxTouchPoints)}`.

## Runtime

- `AppStore.audience()` (`src/agent/store/app-store.ts`) returns `{mobile, desktop}` from the slot row, read at most every 5 s like `callsOn()`. A missing row or missing columns read as both on.
- `PushDevices` gets an optional `audience?(): {mobile: boolean; desktop: boolean}`. Absent, both are on: the devices of the local relay (`src/local/devices.ts`) keep notifying everywhere.
- `Notifier` (`src/agent/push/notifier.ts`):
  - `message()` writes its history row as today, then returns before ntfy and push when both flags are off. `call()`, `missedCall()` and the new `notice()` return at once.
  - New exported `deviceSide(t: PushTarget): "mobile" | "desktop"`: `isPhone(t)` is mobile, else `kind` of the parsed `t.sub` when it is `"mobile"`, else desktop (also when `sub` does not parse). `deliver()` keeps a target when its side is on.
  - ntfy is not part of either side: untouched, except that an account that is off sends none.
  - `alert()` ignores the audience.
- New `Notifier.notice(title, body, urgency = "high")`: like `alert()`, but through the audience. Two callers that push content through `alert()` today move to it: the missed calls of the Activity feed (`src/agent/jobs/missed-calls.ts`) and the findings of a check (`src/agent/commands/check.ts`, "Found by the check"). Every other `alert()` caller is account health and stays.
- `Notify` (the `Pick` in `notifier.ts`) gains `notice`. `ServerNotifier` (`src/local/server-link.ts`) implements it with a new push op.
- `PushBody` (`src/shared/relay-sync.ts`) gains `{op: "notice", title, body, urgency}`. `relayPush` (`src/lib/relay.ts`) handles it with `n.notice(...)`. The `devices` object of `relayNotifier` gains `audience: () => app.audience()`.
- Version skew: a server of an earlier release refuses the `notice` op of a relay (unknown `op` in `PushBody`), for that push only. Deploy the server first, then update the relays. A relay of an earlier release sends those two pushes as `alert` and reaches every device until it is updated.
- Calls: `call()` filters like any push. The `phonesRinging` bookkeeping counts only `fcm:` phones and the Safari skip of the "again" pushes (`APPLE_PUSH`) is unchanged: both work on the endpoint, not on the side.

## App

- New `src/components/NotifyMenu.tsx` (props: account, saving, onChange), built from `DropdownMenu` and `DropdownMenuCheckboxItem` (`src/components/ui/dropdown-menu.tsx`). Trigger: `Button variant="outline"` at `h-10 md:h-9`, `w-36` on wide cards, showing the state as icon and word: `MonitorSmartphoneIcon` Both, `SmartphoneIcon` Mobile, `MonitorIcon` Desktop, `BellOffIcon` Off. `aria-label` is "Notifications of {name}".
- Menu: two items, Mobile with `SmartphoneIcon` and Desktop with `MonitorIcon`, each with a checkbox, `w-48`, `onSelect={(e) => e.preventDefault()}` so it stays open.
- `ui/dropdown-menu.tsx`: the indicator of `DropdownMenuCheckboxItem` becomes a visible 16 px box on the left (checked: `bg-primary` with the check), since the stock indicator is a bare check mark. No other file uses that item.
- `Settings.tsx`: `NotifyMenu` goes in both control groups of a row (the relay one and the other one), before the Calls switch. On a card narrower than 540 px the Status select takes its own line, and the menu button and the Calls switch share the next one.
- A tick updates the account in the list at once, then `patch`es `{notifyMobile}` or `{notifyDesktop}`. An error restores the box and shows `toast.error`. A success toast carries one line, with a fixed id per account so quick changes replace each other: "{name}: mobile and desktop", "mobile only", "desktop only", "notifications off".
- The Teams accounts description gets one sentence: tick Mobile, Desktop or both, and with none ticked the account sends no notifications. The "Advanced Teams settings" summary is renamed as above.

## Error handling

- PATCH with a non-boolean: 400, nothing changed.
- PATCH on a slot of another user: refused by `requireAccount`, as for `calls`.
- Save fails in the browser: the box is restored, the error is shown.
- `app.db` unreadable from an agent: `audience()` returns both on, so a notification is never lost to a read error.

## Testing

Vitest, next to `test/light-mode.test.ts`:

- migration adds both columns with default 1 on an existing `app.db`;
- `setNotify` writes one column without touching the other;
- route: 400 for a non-boolean, the values stored, a foreign slot refused;
- `AppStore.audience()`: defaults, stored values, the 5 s cache;
- `deviceKindOf`: iPhone, iPad (as a Mac with touch points), Android Chrome are mobile; Windows Chrome, Mac Safari with no touch points are desktop;
- `subscriptionOf` stores `kind` from `device`, defaults to desktop when absent or unknown, and the stored JSON still carries `endpoint` and `keys` unchanged;
- `deviceSide`: `fcm:` is mobile, a subscription of kind mobile is mobile, a subscription without `kind` or with unparsable `sub` is desktop;
- `Notifier` with fake devices, fake Web Push and fake FCM: Both, Mobile only, Desktop only, Off, for `message`, `call` (ringing, again, ended), `missedCall` and `notice`, with a phone of the Android app, a mobile browser and a desktop browser among the devices; the history row written and no push or ntfy when off; `alert` delivered in every mode;
- `PushBody` accepts `notice`; `relayPush` routes it through the audience of the slot;
- account summary carries both flags, and reads absent flags as on;
- `pushMissedCalls` and the check summary use `notice`.

Before the PR: `npm test`, the typecheck and `next build` of `teams-relay/app`. Then Settings open in a browser on a local build: the menu opens, ticks save and survive a reload, both unticked shows Off, and the row holds on a 360 px window.

## Out of scope

- Select-all per column.
- Re-registering existing subscriptions on their own: a device takes its kind when it taps Enable notifications again.
- Per-chat or per-event-type rules.
- Quiet hours.
- Any change to the Android app, its channels or its permissions.
- Lifting the Teams accounts card out of the "Advanced Teams settings" fold.
