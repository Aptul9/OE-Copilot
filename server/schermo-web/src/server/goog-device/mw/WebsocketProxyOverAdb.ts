import { WebsocketProxy } from '../../mw/WebsocketProxy';
import { AdbUtils } from '../AdbUtils';
import WS from 'ws';
import { RequestParameters } from '../../mw/Mw';
import { ACTION } from '../../../common/Action';
import { SERVER_PORT } from '../../../common/Constants';

// Il solo tunnel che serve: dal browser al server di scrcpy sul telefono (tcp:8886). Una porta qualunque
// del telefono (adbd compreso) non si raggiunge da qui.
const REMOTE = `tcp:${SERVER_PORT}`;

export class WebsocketProxyOverAdb extends WebsocketProxy {
    public static processRequest(ws: WS, params: RequestParameters): WebsocketProxy | undefined {
        const { action, url } = params;
        if (action !== ACTION.PROXY_ADB) {
            return;
        }
        const remote = url.searchParams.get('remote');
        const udid = url.searchParams.get('udid');
        if (remote !== REMOTE) {
            ws.close(4003, `[${this.TAG}] Invalid value for "remote" parameter`);
            return;
        }
        if (typeof udid !== 'string' || !udid) {
            ws.close(4003, `[${this.TAG}] Invalid value for "udid" parameter`);
            return;
        }
        return this.createProxyOverAdb(ws, udid, remote);
    }

    public static createProxyOverAdb(ws: WS, udid: string, remote: string): WebsocketProxy {
        const service = new WebsocketProxy(ws);
        AdbUtils.forward(udid, remote)
            .then((port) => {
                return service.init(`ws://127.0.0.1:${port}`);
            })
            .catch((e) => {
                const msg = `[${this.TAG}] Failed to start service: ${e.message}`;
                console.error(msg);
                ws.close(4005, msg);
            });
        return service;
    }
}
