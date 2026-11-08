import * as portfinder from 'portfinder';
import { AdbForward, createClient } from './adb';

export class AdbUtils {
    public static async forward(serial: string, remote: string): Promise<number> {
        const device = createClient().getDevice(serial);
        const forwards = await device.listForwards();
        const forward = forwards.find((item: AdbForward) => {
            return item.remote === remote && item.local.startsWith('tcp:') && item.serial === serial;
        });
        if (forward) {
            const { local } = forward;
            return parseInt(local.split('tcp:')[1], 10);
        }
        const port = await portfinder.getPortPromise();
        const local = `tcp:${port}`;
        await device.forward(local, remote);
        return port;
    }
}
