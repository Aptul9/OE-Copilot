import Database from "better-sqlite3";
import { appDb } from "./appdb";
import { slotDbPath } from "./slotdb";

export type SavedSlot = { slot: number; snapshot: string } | { slot: number; error: string };

// The query Everflow ran on each slot store over SSH (storeQuery in everflow src/watchers/relay.ts): one JSON text with
// the account (state me, last_scan_ts, state), the muted chats and the saved messages with a numeric id at or after
// `since`, oldest first, text cut at 2000 characters. One row more than Everflow's cap, so Everflow sees it was hit.
const STORE_QUERY = `SELECT json_object('state',json((SELECT json_group_object(k,v) FROM state WHERE k IN ('me','last_scan_ts','state'))),'muted',json((SELECT json_group_array(name) FROM chats WHERE muted=1)),'messages',json((SELECT json_group_array(json_object('chat',chat,'mid',mid,'author',author,'text',text,'mine',mine,'extra',extra)) FROM (SELECT chat,mid,author,substr(text,1,2000) AS text,mine,extra FROM chat_messages WHERE mid GLOB '[0-9]*' AND mid NOT GLOB '*[^0-9]*' AND CAST(mid AS INTEGER)>=? ORDER BY CAST(mid AS INTEGER),chat LIMIT ?)))) AS snapshot`;

// Saved messages of every active Teams account of a user since a time (ms), for Everflow. A store that cannot be read
// is that slot's error; the others are still read.
export function savedSince(userId: string, since: number, maxRows: number): { slots: SavedSlot[] } {
  const ids = appDb().prepare("SELECT slot FROM teams_accounts WHERE owner_id=? AND stopped=0 ORDER BY slot LIMIT 1025").pluck().all(userId) as number[];
  return { slots: ids.map((slot) => readStore(slot, since, maxRows)) };
}

function readStore(slot: number, since: number, maxRows: number): SavedSlot {
  let db: Database.Database | null = null;
  try {
    db = new Database(slotDbPath(slot), { readonly: true, fileMustExist: true });
    // short: every active slot is read in turn within the 60 s the communication service waits
    db.pragma("busy_timeout = 2000");
    db.pragma("query_only = ON");
    db.pragma("trusted_schema = OFF");
    const snapshot = db.prepare(STORE_QUERY).pluck().get(since, maxRows + 1);
    if (typeof snapshot !== "string") throw new Error("no snapshot");
    return { slot, snapshot };
  } catch {
    return { slot, error: "store-unavailable" };
  } finally {
    db?.close();
  }
}
