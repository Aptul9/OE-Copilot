import { StreamReceiver } from '../../client/StreamReceiver';
import { ParamsStreamScrcpy } from '../../../types/ParamsStreamScrcpy';
import { ACTION } from '../../../common/Action';
import Util from '../../Util';
import { defaultStreamUrl } from '../StreamUrl';

export class StreamReceiverScrcpy extends StreamReceiver<ParamsStreamScrcpy> {
    public static parseParameters(params: URLSearchParams): ParamsStreamScrcpy {
        const typedParams = super.parseParameters(params);
        const { action } = typedParams;
        if (action !== ACTION.STREAM_SCRCPY) {
            throw Error('Incorrect action');
        }
        const udid = Util.parseString(params, 'udid', true);
        return {
            ...typedParams,
            action,
            udid,
            ws: params.get('ws') || defaultStreamUrl(udid),
            player: Util.parseString(params, 'player', true),
        };
    }
    protected buildDirectWebSocketUrl(): URL {
        const url = new URL((this.params as ParamsStreamScrcpy).ws);
        if ((url.protocol !== 'ws:' && url.protocol !== 'wss:') || url.host !== location.host) {
            throw Error('The stream must come from the same site as the page');
        }
        return url;
    }
}
