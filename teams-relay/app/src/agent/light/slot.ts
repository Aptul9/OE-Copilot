import { loadConfig } from "../config";
import { log } from "../log";
import { NewMessageDetector } from "../logic/new-messages";
import { Media } from "../media";
import { FcmSender, loadServiceAccount } from "../push/fcm";
import { Notifier } from "../push/notifier";
import { loadVapidKeys } from "../push/vapid";
import { AppStore } from "../store/app-store";
import { SlotStore } from "../store/slot-store";
import { LightApi } from "./api";
import type { Light } from "./context";
import { runLight } from "./loop";
import { StatusCollector } from "./status";
import { LightToken } from "./token";

// One light slot, on the environment given: runs until `signal` aborts (without one, until the process ends)
export async function startSlot(env: Record<string, string | undefined>, signal?: AbortSignal): Promise<void> {
  const config = loadConfig(env);
  const store = SlotStore.open(config.dbPath);
  const vapid = loadVapidKeys(config.vapid.privateKeyFile, config.vapid.appKeyFile);
  if (!vapid) log.warn("push", "no VAPID private key: push notifications off", { file: config.vapid.privateKeyFile });
  const devices = new AppStore(config.appDb, config.slot);
  const sa = loadServiceAccount(config.fcmCredentials);
  const notifier = new Notifier({ store, devices, vapid, subject: config.vapid.subject, ntfy: config.ntfy, fcm: sa ? new FcmSender(sa) : null, answerable: config.nativeCalls });
  const token = new LightToken(config.lightTokenFile, Date.now, config.sharedBroker);
  const api = new LightApi(token, () => token.region());
  const l: Light = {
    config,
    store,
    notifier,
    media: new Media(config.mediaDir, config.filesDir),
    detector: new NewMessageDetector(),
    api,
    token,
    me: { oid: "", name: "" },
    list: { ids: new Map(), names: new Map(), members: new Map() },
    people: new Map(),
    authorsRead: new Map(),
    mentioned: new Set(),
    due: { list: 0, activity: 0 },
    scanAt: 0,
    error: "",
    signedOut: false,
    health: null,
    startedAt: Date.now(),
    feedRead: false,
    callsWanted: () => devices.callsOn(),
    status: new StatusCollector({ pages: () => devices.statusPages(), file: config.statusDb, slot: config.slot, gap: 3 * config.lightPoll }),
  };
  log.info("light", "start", { slot: config.slot, poll: config.lightPoll, push: !!vapid, fcm: !!sa, signIn: !!token.stored() });
  // the database stays open after a stop in the light host: the calling worker's exit and pushes still in flight write
  // to it, and a closed one would throw out of them and end every slot of the process
  await runLight(l, signal);
}
