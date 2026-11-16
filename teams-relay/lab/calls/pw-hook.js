// Playwriter: open Teams web in a new tab of this profile with RTCPeerConnection wrapped (every call, SDP and state
// change into window.__pcLog) and the calling HTTP requests and Trouter frames recorded through CDP into state.net.
const hook = `(() => {
  if (window.__pcHook) return; window.__pcHook = true;
  const log = (window.__pcLog = window.__pcLog || []);
  const O = window.RTCPeerConnection;
  if (!O) return;
  let n = 0;
  window.__pcs = [];
  const now = () => Date.now();
  function P(...args) {
    const pc = new O(...args);
    const id = ++n;
    window.__pcs.push(pc);
    const c = args[0] || {};
    log.push({ t: now(), id, ev: 'new', cfg: JSON.stringify({ ...c, iceServers: (c.iceServers || []).map((s) => ({ urls: s.urls })) }) });
    for (const m of ['createOffer', 'createAnswer', 'setLocalDescription', 'setRemoteDescription', 'addIceCandidate', 'close', 'restartIce', 'setConfiguration']) {
      const f = pc[m].bind(pc);
      pc[m] = async (...a) => {
        const rec = { t: now(), id, ev: m };
        if (m === 'setLocalDescription' || m === 'setRemoteDescription') { rec.type = a[0] && a[0].type; rec.sdp = a[0] && a[0].sdp; }
        if (m === 'addIceCandidate') rec.cand = a[0] && (a[0].candidate !== undefined ? a[0].candidate : JSON.stringify(a[0]));
        log.push(rec);
        try {
          const r = await f(...a);
          if (m === 'createOffer' || m === 'createAnswer') log.push({ t: now(), id, ev: m + ':done', type: r && r.type, sdp: r && r.sdp });
          return r;
        } catch (e) { log.push({ t: now(), id, ev: m + ':error', err: String(e) }); throw e; }
      };
    }
    pc.addEventListener('icecandidate', (e) => log.push({ t: now(), id, ev: 'icecandidate', cand: e.candidate && e.candidate.candidate }));
    pc.addEventListener('iceconnectionstatechange', () => log.push({ t: now(), id, ev: 'ice', s: pc.iceConnectionState }));
    pc.addEventListener('connectionstatechange', () => log.push({ t: now(), id, ev: 'conn', s: pc.connectionState }));
    pc.addEventListener('signalingstatechange', () => log.push({ t: now(), id, ev: 'sig', s: pc.signalingState }));
    pc.addEventListener('icegatheringstatechange', () => log.push({ t: now(), id, ev: 'gather', s: pc.iceGatheringState }));
    return pc;
  }
  P.prototype = O.prototype;
  Object.setPrototypeOf(P, O);
  window.RTCPeerConnection = P;
  window.webkitRTCPeerConnection = P;
  setInterval(async () => {
    for (let i = 0; i < window.__pcs.length; i++) {
      const pc = window.__pcs[i];
      if (pc.connectionState === 'closed') continue;
      try {
        const st = await pc.getStats();
        const out = [];
        st.forEach((r) => {
          if (r.type === 'transport') out.push({ type: r.type, dtls: r.dtlsState, role: r.iceRole, dtlsRole: r.dtlsRole, ice: r.iceState, sel: r.selectedCandidatePairId, srtp: r.srtpCipher, tls: r.tlsVersion });
          if (r.type === 'candidate-pair' && (r.nominated || r.state === 'succeeded' || r.requestsSent > 0)) out.push({ type: 'pair', id: r.id, st: r.state, nom: r.nominated, l: r.localCandidateId, r: r.remoteCandidateId, rq: r.requestsSent, rs: r.responsesReceived, rqr: r.requestsReceived, rss: r.responsesSent });
          if (r.type === 'local-candidate' || r.type === 'remote-candidate') out.push({ type: r.type, id: r.id, ct: r.candidateType, a: r.address, p: r.port, proto: r.protocol, relay: r.relayProtocol });
          if (r.type === 'inbound-rtp' || r.type === 'outbound-rtp') out.push({ type: r.type, kind: r.kind, pk: r.packetsReceived || r.packetsSent, b: r.bytesReceived || r.bytesSent });
        });
        window.__pcLog.push({ t: Date.now(), id: i + 1, ev: 'stats', s: out });
      } catch (e) {}
    }
  }, 1000);
})();`;

state.hook = hook;
state.net = [];
state.page = await context.newPage();
const cdp = await getCDPSession({ page: state.page });
state.cdp = cdp;
await cdp.send('Page.enable');
await cdp.send('Page.addScriptToEvaluateOnNewDocument', { source: hook, runImmediately: true });
await cdp.send('Network.enable', { maxPostDataSize: 200000 });
const want = (u) => /flightproxy|cc\.skype\.com|conv\.skype|epconv|callAgent|trouter|registrar|relay\.teams|mdn|trap|calling/i.test(u);
cdp.on('Network.requestWillBeSent', (e) => {
  if (want(e.request.url)) state.net.push({ t: Date.now(), ev: 'req', id: e.requestId, m: e.request.method, url: e.request.url, h: e.request.headers, body: e.request.postData });
});
cdp.on('Network.responseReceived', (e) => {
  if (want(e.response.url)) state.net.push({ t: Date.now(), ev: 'res', id: e.requestId, url: e.response.url, status: e.response.status });
});
cdp.on('Network.webSocketCreated', (e) => state.net.push({ t: Date.now(), ev: 'ws', id: e.requestId, url: e.url }));
cdp.on('Network.webSocketFrameReceived', (e) => state.net.push({ t: Date.now(), ev: 'wsin', id: e.requestId, d: e.response.payloadData }));
cdp.on('Network.webSocketFrameSent', (e) => state.net.push({ t: Date.now(), ev: 'wsout', id: e.requestId, d: e.response.payloadData }));
await state.page.goto('https://teams.microsoft.com/v2/', { waitUntil: 'domcontentloaded' });
await state.page.waitForTimeout(8000);
console.log('URL:', state.page.url());
console.log('hooked:', await state.page.evaluate(() => !!window.__pcHook));
console.log('net so far:', state.net.length);
