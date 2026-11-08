import '../../LICENSE';
import { HttpServer } from './services/HttpServer';
import { WebSocketServer } from './services/WebSocketServer';
import { Service } from './services/Service';
import { WebsocketMultiplexer } from './mw/WebsocketMultiplexer';
import { ControlCenter } from './goog-device/services/ControlCenter';
import { DeviceTracker } from './goog-device/mw/DeviceTracker';
import { WebsocketProxyOverAdb } from './goog-device/mw/WebsocketProxyOverAdb';

WebsocketMultiplexer.registerMw(DeviceTracker);

const runningServices: Service[] = [HttpServer.getInstance(), WebSocketServer.getInstance(), ControlCenter.getInstance()];

Promise.all(runningServices.map((service) => service.start()))
    .then(() => {
        const wsService = WebSocketServer.getInstance();
        [DeviceTracker, WebsocketMultiplexer, WebsocketProxyOverAdb].forEach((mw) => wsService.registerMw(mw));
        process.on('SIGINT', exit);
        process.on('SIGTERM', exit);
    })
    .catch((error) => {
        console.error(error.message);
        exit('1');
    });

let interrupted = false;
function exit(signal: string) {
    console.log(`\nReceived signal ${signal}`);
    if (interrupted) {
        console.log('Force exit');
        process.exit(0);
        return;
    }
    interrupted = true;
    runningServices.forEach((service: Service) => {
        console.log(`Stopping ${service.getName()} ...`);
        service.release();
    });
}
