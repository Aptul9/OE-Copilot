import * as portfinder from 'portfinder';
import { AdbForward, createClient } from './adb';

export class AdbUtils {
    public static async forward(serial: string, remote: string): Promise<number> {
        const client = createClient();
        const forwards = await client.listForwards(serial);
        const forward = forwards.find((item: AdbForward) => {
            return item.remote === remote && item.local.startsWith('tcp:') && item.serial === serial;
        });
        if (forward) {
            const { local } = forward;
            return parseInt(local.split('tcp:')[1], 10);
        }
        const port = await portfinder.getPortPromise();
        const local = `tcp:${port}`;
        await client.forward(serial, local, remote);
        return port;
    }
}
