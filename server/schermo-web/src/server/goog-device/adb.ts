// Il solo punto che conosce la libreria che parla con adb: il resto del server usa questi nomi.
import { Adb, Client, Forward, TrackerChangeSet } from '@devicefarmer/adbkit';
import type Tracker from '@devicefarmer/adbkit/dist/src/adb/tracker';
import type PushTransfer from '@devicefarmer/adbkit/dist/src/adb/sync/pushtransfer';

export type AdbClient = Client;
export type AdbTracker = Tracker;
export type AdbPushTransfer = PushTransfer;
export type AdbTrackerChangeSet = TrackerChangeSet;
export type AdbForward = Forward;

export const readAll = Adb.util.readAll;

export function createClient(): AdbClient {
    const port = parseInt(process.env.ADB_PORT || '', 10);
    return Adb.createClient({
        host: process.env.ADB_HOST || '127.0.0.1',
        port: isNaN(port) ? 5037 : port,
    });
}
