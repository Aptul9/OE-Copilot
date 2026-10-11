// Il solo punto che conosce la libreria che parla con adb: il resto del server usa questi nomi.
import Adb from '@dead50f7/adbkit/lib/adb';
import Client from '@dead50f7/adbkit/lib/adb/client';
import Tracker from '@dead50f7/adbkit/lib/adb/tracker';
import PushTransfer from '@dead50f7/adbkit/lib/adb/sync/pushtransfer';
import { TrackerChangeSet } from '@dead50f7/adbkit/lib/TrackerChangeSet';
import { Forward } from '@dead50f7/adbkit/lib/Forward';

export type AdbClient = Client;
export type AdbTracker = Tracker;
export type AdbPushTransfer = PushTransfer;
export type AdbTrackerChangeSet = TrackerChangeSet;
export type AdbForward = Forward;

export const readAll = Adb.util.readAll;

export function createClient(): AdbClient {
    const port = parseInt(process.env.ADB_PORT || '', 10);
    return new Client({
        host: process.env.ADB_HOST || '127.0.0.1',
        port: isNaN(port) ? 5037 : port,
    });
}
