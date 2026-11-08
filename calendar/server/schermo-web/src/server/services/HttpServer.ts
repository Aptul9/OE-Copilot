import * as http from 'http';
import path from 'path';
import { Service } from './Service';
import express from 'express';
import { TypedEmitter } from '../../common/TypedEmitter';

const PUBLIC_DIR = path.join(__dirname, './public');
const PORT = parseInt(process.env.PORT || '', 10) || 8000;

export type ServerAndPort = {
    server: http.Server;
    port: number;
};

interface HttpServerEvents {
    started: boolean;
}

// Solo HTTP: davanti c'è il Caddy che mette HTTPS e chiede l'accesso, il servizio non è pubblicato.
export class HttpServer extends TypedEmitter<HttpServerEvents> implements Service {
    private static instance: HttpServer;
    private servers: ServerAndPort[] = [];
    private started = false;

    protected constructor() {
        super();
    }

    public static getInstance(): HttpServer {
        if (!this.instance) {
            this.instance = new HttpServer();
        }
        return this.instance;
    }

    public static hasInstance(): boolean {
        return !!this.instance;
    }

    public async getServers(): Promise<ServerAndPort[]> {
        if (this.started) {
            return [...this.servers];
        }
        return new Promise<ServerAndPort[]>((resolve) => {
            this.once('started', () => {
                resolve([...this.servers]);
            });
        });
    }

    public getName(): string {
        return `HTTP Server Service`;
    }

    public async start(): Promise<void> {
        const app = express();
        app.disable('x-powered-by');
        app.get('/salute', (_req, res) => {
            res.type('text/plain').send('ok');
        });
        app.use(express.static(PUBLIC_DIR));
        const server = http.createServer(app);
        this.servers.push({ server, port: PORT });
        server.listen(PORT, () => {
            console.log(`Listening on http://0.0.0.0:${PORT}`);
        });
        this.started = true;
        this.emit('started', true);
    }

    public release(): void {
        this.servers.forEach((item) => {
            item.server.close();
        });
    }
}
