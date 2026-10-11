import { ACTION } from '../../common/Action';
import { SERVER_PORT } from '../../common/Constants';

// Dove sta il flusso di un telefono: lo stesso sito e lo stesso percorso della pagina, tramite adb. Un indirizzo che
// porta solo `action=stream&udid=...` (il tasto "Telefono" dell'app) non ha bisogno del parametro `ws`.
export function defaultStreamUrl(udid: string): string {
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const url = new URL(`${protocol}//${location.host}${location.pathname}`);
    url.searchParams.set('action', ACTION.PROXY_ADB);
    url.searchParams.set('remote', `tcp:${SERVER_PORT}`);
    url.searchParams.set('udid', udid);
    return url.toString();
}
