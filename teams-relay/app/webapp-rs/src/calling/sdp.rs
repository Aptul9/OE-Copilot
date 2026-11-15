//! The SDP dialect of Teams calling (content type application/sdp-ngc-1.0) and the standard SDP of WebRTC. The Teams
//! web stack turns the browser's offer into that dialect before it goes to the call controller, and the answer back:
//! RTP/SAVP for UDP/TLS/RTP/SAVPF, the relay candidate as the address of the stream, the SSRC as a range, a label per
//! stream, TCP candidates as tcp-act/tcp-pass, the URIs of some header extensions with backslashes. Only audio is
//! offered here: the video streams and the data stream are declined in the offer, as an audio-only client does.

/// The lines of an SDP, without their line ends.
fn lines(sdp: &str) -> Vec<&str> {
    sdp.split('\n').map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty()).collect()
}

/// The session part and each media section of an SDP, as lines.
fn sections(sdp: &str) -> (Vec<&str>, Vec<Vec<&str>>) {
    let mut session = vec![];
    let mut media: Vec<Vec<&str>> = vec![];
    for l in lines(sdp) {
        if l.starts_with("m=") {
            media.push(vec![l]);
        } else if let Some(m) = media.last_mut() {
            m.push(l);
        } else {
            session.push(l);
        }
    }
    (session, media)
}

fn attr<'a>(section: &[&'a str], name: &str) -> Option<&'a str> {
    let prefix = format!("a={name}:");
    section.iter().find_map(|l| l.strip_prefix(prefix.as_str()))
}

fn attrs<'a>(section: &[&'a str], name: &str) -> Vec<&'a str> {
    let prefix = format!("a={name}:");
    section.iter().filter_map(|l| l.strip_prefix(prefix.as_str())).collect()
}

/// A candidate line (a=candidate:...) as its fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub foundation: String,
    pub component: String,
    pub transport: String,
    pub priority: u64,
    pub ip: String,
    pub port: u16,
    pub kind: String,
    pub raddr: Option<(String, u16)>,
    pub tcptype: Option<String>,
}

impl Candidate {
    pub fn parse(value: &str) -> Option<Candidate> {
        let f: Vec<&str> = value.split_whitespace().collect();
        if f.len() < 8 || f[6] != "typ" {
            return None;
        }
        let mut c = Candidate {
            foundation: f[0].into(),
            component: f[1].into(),
            transport: f[2].to_lowercase(),
            priority: f[3].parse().ok()?,
            ip: f[4].into(),
            port: f[5].parse().ok()?,
            kind: f[7].into(),
            raddr: None,
            tcptype: None,
        };
        let mut i = 8;
        while i + 1 < f.len() {
            match f[i] {
                "raddr" => c.raddr = Some((f[i + 1].into(), f.get(i + 3).and_then(|p| p.parse().ok()).unwrap_or(0))),
                "tcptype" => c.tcptype = Some(f[i + 1].into()),
                _ => {}
            }
            i += if f[i] == "raddr" { 4 } else { 2 };
        }
        // the dialect writes the TCP type into the transport
        match c.transport.as_str() {
            "tcp-act" => (c.transport, c.tcptype) = ("tcp".into(), Some("active".into())),
            "tcp-pass" => (c.transport, c.tcptype) = ("tcp".into(), Some("passive".into())),
            "tcp-so" => (c.transport, c.tcptype) = ("tcp".into(), Some("so".into())),
            _ => {}
        }
        Some(c)
    }

    fn tail(&self) -> String {
        self.raddr.as_ref().map(|(a, p)| format!(" raddr {a} rport {p}")).unwrap_or_default()
    }

    /// As the dialect writes it.
    pub fn ngc(&self) -> String {
        let transport = match (self.transport.as_str(), self.tcptype.as_deref()) {
            ("tcp", Some("active")) => "tcp-act".to_string(),
            ("tcp", Some("passive")) => "tcp-pass".to_string(),
            (t, _) => t.to_string(),
        };
        format!("{} {} {} {} {} {} typ {}{}", self.foundation, self.component, transport, self.priority, self.ip, self.port, self.kind, self.tail())
    }

    /// As standard SDP writes it.
    pub fn standard(&self) -> String {
        let tcp = self.tcptype.as_ref().map(|t| format!(" tcptype {t}")).unwrap_or_default();
        format!("{} {} {} {} {} {} typ {}{}{tcp}", self.foundation, self.component, self.transport, self.priority, self.ip, self.port, self.kind, self.tail())
    }
}

/// The address the stream is announced at in the dialect: the relay candidate, else the reflexive one, else a host
/// address (as the Teams stack picks it).
fn default_address(candidates: &[Candidate]) -> Option<&Candidate> {
    let udp = |kind: &str| candidates.iter().filter(|c| c.transport == "udp" && c.kind == kind && c.ip.contains('.')).max_by_key(|c| c.priority);
    udp("relay").or_else(|| udp("srflx")).or_else(|| udp("host"))
}

/// Header extensions whose URI the dialect writes with backslashes.
const BACKSLASHED: [&str; 2] =
    ["http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time", "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"];

fn ngc_extmap(value: &str) -> String {
    let (id, uri) = value.split_once(' ').unwrap_or((value, ""));
    if BACKSLASHED.contains(&uri) { format!("{id} {}", uri.replace('/', "\\")) } else { value.to_string() }
}

fn standard_extmap(value: &str) -> String {
    let (id, uri) = value.split_once(' ').unwrap_or((value, ""));
    if uri.starts_with("http:\\") { format!("{id} {}", uri.replace('\\', "/")) } else { value.to_string() }
}

/// What the dialect offer needs besides the standard offer: the label of the stream and its SSRC.
pub struct OfferExtras {
    pub ssrc: u32,
    pub cname: String,
}

/// The standard offer of the peer connection (ICE gathered) in the dialect, as an audio-only Teams client sends it.
/// None when it has no audio or no candidate.
pub fn to_ngc(offer: &str, extras: &OfferExtras) -> Option<String> {
    ngc(offer, extras, None)
}

/// The standard answer of the peer connection to `offer` (a dialect offer) in the dialect. An answer has the m-lines of
/// the offer in its order, with its mids: Teams desktop and mobile offer audio_0 and video_1 and end the call on any
/// other shape (callEnd 406/4122 FinalAnswerError, 2026-11-15); the streams other than audio are declined (port 0).
pub fn to_ngc_answer(answer: &str, extras: &OfferExtras, offer: &str) -> Option<String> {
    ngc(answer, extras, Some(offer))
}

/// A stream of the offer an audio-only answer declines: its m-line with port 0, its mid and label.
fn declined(section: &[&str]) -> Vec<String> {
    let mut f: Vec<&str> = section[0].split_whitespace().collect();
    if f.len() > 1 {
        f[1] = "0";
    }
    let mut o = vec![f.join(" ")];
    for name in ["mid", "label"] {
        if let Some(v) = attr(section, name) {
            o.push(format!("a={name}:{v}"));
        }
    }
    o
}

fn ngc(local: &str, extras: &OfferExtras, offer: Option<&str>) -> Option<String> {
    // the m-lines around the audio: those of the offer answered, else the streams an audio-only Teams web offer declines
    let offered = offer.map(sections).map(|(_, m)| m).unwrap_or_default();
    let audio_at = offered.iter().position(|m| m[0].starts_with("m=audio ") && !m[0].starts_with("m=audio 0 "));
    let mid = audio_at.and_then(|i| attr(&offered[i], "mid")).unwrap_or("0");
    let (session, media) = sections(local);
    let audio = media.iter().find(|m| m[0].starts_with("m=audio "))?;
    let candidates: Vec<Candidate> = attrs(audio, "candidate").into_iter().chain(attrs(&session, "candidate")).filter_map(Candidate::parse).collect();
    let at = default_address(&candidates)?;
    let from = |name: &str| attr(audio, name).or_else(|| attr(&session, name));
    let pts: Vec<&str> = audio[0].split_whitespace().skip(3).collect();
    let origin = session.iter().find(|l| l.starts_with("o=")).copied().unwrap_or("o=- 0 2 IN IP4 127.0.0.1");
    let mut o = vec![
        "v=0".to_string(),
        origin.to_string(),
        "s=-".into(),
        "b=CT:7000".into(),
        "t=0 0".into(),
        "a=msid-semantic: WMS *".into(),
        format!("a=group:BUNDLE {mid}"),
    ];
    if let Some(i) = audio_at {
        for m in &offered[..i] {
            o.extend(declined(m));
        }
    }
    o.extend([
        format!("m=audio {} RTP/SAVP {}", at.port, pts.join(" ")),
        format!("c=IN IP4 {}", at.ip),
        "a=x-signaling-fb:* x-message app recv:dsh".into(),
        format!("a=x-ssrc-range:{0}-{0}", extras.ssrc),
    ]);
    for pt in &pts {
        if let Some(map) = attrs(audio, "rtpmap").into_iter().find(|m| m.split_whitespace().next() == Some(*pt)) {
            o.push(format!("a=rtpmap:{map}"));
        }
    }
    for pt in &pts {
        if let Some(fmtp) = attrs(audio, "fmtp").into_iter().find(|m| m.split_whitespace().next() == Some(*pt)) {
            o.push(format!("a=fmtp:{fmtp}"));
        }
    }
    o.push(format!("a=rtcp:{}", at.port));
    for e in attrs(audio, "extmap") {
        o.push(format!("a=extmap:{}", ngc_extmap(e)));
    }
    o.push(format!("a=setup:{}", from("setup").unwrap_or("actpass")));
    o.push(format!("a=mid:{mid}"));
    o.push("a=sendrecv".into());
    o.push(format!("a=ice-ufrag:{}", from("ice-ufrag")?));
    o.push(format!("a=ice-pwd:{}", from("ice-pwd")?));
    o.push(format!("a=fingerprint:{}", from("fingerprint")?));
    for c in candidates.iter().filter(|c| c.transport == "udp") {
        o.push(format!("a=candidate:{}", c.ngc()));
    }
    o.push("a=ice-options:trickle".into());
    o.push(format!("a=ssrc:{} cname:{}", extras.ssrc, extras.cname));
    o.push("a=rtcp-mux".into());
    o.push("a=label:main-audio".into());
    match audio_at {
        Some(i) => {
            for m in &offered[i + 1..] {
                o.extend(declined(m));
            }
        }
        // the streams an audio-only client declines
        None => {
            for (m, label) in
                [("m=video 0 RTP/SAVP 39 40", "main-video"), ("m=video 0 RTP/SAVP 39 40", "applicationsharing-video"), ("m=x-data 0 RTP/SAVP 127 126", "data")]
            {
                o.push(m.into());
                o.push(format!("a=label:{label}"));
            }
        }
    }
    Some(o.join("\r\n") + "\r\n")
}

/// The audio of a dialect answer: the remote SSRC (x-ssrc-range) and the payload types in the order the far end
/// prefers them.
#[derive(Debug, PartialEq)]
pub struct Answered {
    pub sdp: String,
    pub ssrc: Option<u32>,
    pub payload_types: Vec<u8>,
}

/// A dialect answer as the standard SDP of an audio-only peer connection whose audio has mid "0". None when it accepts
/// no audio.
pub fn from_ngc(ngc: &str) -> Option<Answered> {
    standard(ngc, "active")
}

/// A dialect offer as the standard SDP of an audio-only peer connection. The offers of Teams desktop and mobile carry
/// no a=setup: they take either DTLS role, as actpass says.
pub fn from_ngc_offer(ngc: &str) -> Option<Answered> {
    standard(ngc, "actpass")
}

fn standard(ngc: &str, setup: &str) -> Option<Answered> {
    let (session, media) = sections(ngc);
    let audio = media.iter().find(|m| m[0].starts_with("m=audio ") && !m[0].starts_with("m=audio 0 "))?;
    let m: Vec<&str> = audio[0].split_whitespace().collect();
    let port = m.get(1)?;
    let pts: Vec<&str> = m.iter().skip(3).copied().collect();
    let c = audio.iter().chain(session.iter()).find_map(|l| l.strip_prefix("c="))?;
    let from = |name: &str| attr(audio, name).or_else(|| attr(&session, name));
    let ssrc = from("x-ssrc-range").and_then(|r| r.split('-').next()).and_then(|s| s.parse().ok());
    let mut o = vec![
        "v=0".to_string(),
        "o=- 1 0 IN IP4 127.0.0.1".into(),
        "s=-".into(),
        "t=0 0".into(),
        "a=group:BUNDLE 0".into(),
        "a=msid-semantic: WMS *".into(),
        format!("m=audio {port} UDP/TLS/RTP/SAVPF {}", pts.join(" ")),
        format!("c={c}"),
    ];
    if let Some(rtcp) = attr(audio, "rtcp") {
        o.push(format!("a=rtcp:{rtcp}"));
    }
    o.push(format!("a=ice-ufrag:{}", from("ice-ufrag")?));
    o.push(format!("a=ice-pwd:{}", from("ice-pwd")?));
    if session.iter().chain(audio.iter()).any(|l| *l == "a=ice-lite") {
        o.push("a=ice-lite".into());
    }
    o.push(format!("a=fingerprint:{}", from("fingerprint")?));
    o.push(format!("a=setup:{}", from("setup").unwrap_or(setup)));
    o.push("a=mid:0".into());
    let direction = audio.iter().find_map(|l| ["a=sendrecv", "a=sendonly", "a=recvonly", "a=inactive"].into_iter().find(|d| l == d)).unwrap_or("a=sendrecv");
    o.push(direction.into());
    o.push("a=rtcp-mux".into());
    for map in attrs(audio, "rtpmap") {
        o.push(format!("a=rtpmap:{map}"));
    }
    for fmtp in attrs(audio, "fmtp") {
        o.push(format!("a=fmtp:{fmtp}"));
    }
    if let Some(p) = attr(audio, "ptime") {
        o.push(format!("a=ptime:{p}"));
    }
    for e in attrs(audio, "extmap") {
        o.push(format!("a=extmap:{}", standard_extmap(e)));
    }
    for c in attrs(audio, "candidate").into_iter().filter_map(Candidate::parse) {
        o.push(format!("a=candidate:{}", c.standard()));
    }
    o.push("a=end-of-candidates".into());
    if let Some(s) = ssrc {
        o.push(format!("a=ssrc:{s} cname:teams-{s}"));
    }
    Some(Answered { sdp: o.join("\r\n") + "\r\n", ssrc, payload_types: pts.iter().filter_map(|p| p.parse().ok()).collect() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The answer of Microsoft's test-call service in the trace of 2026-11-15 (slot 1, echo call).
    const ECHO_ANSWER: &str = "v=0\r\no=- 207904 0 IN IP4 127.0.0.1\r\ns=session\r\nc=IN IP4 52.112.232.99\r\nb=CT:10000000\r\nt=0 0\r\nm=audio 50497 RTP/SAVP 120 9 0 8 13 101\r\nc=IN IP4 52.112.232.99\r\na=x-signaling-fb:* x-message app send:dsh\r\na=rtcp:50497\r\na=setup:passive\r\na=ice-ufrag:/Nyf\r\na=ice-pwd:UVOLpLaFP8Xh3NyMMleCrOhx\r\na=rtcp-mux\r\na=candidate:1 1 UDP 2130705918 52.112.232.99 50497 typ srflx raddr 10.128.65.160 rport 50497\r\na=candidate:2 1 tcp-act 2121006078 52.112.232.99 50001 typ srflx raddr 10.128.65.160 rport 50001\r\na=candidate:3 1 UDP 1258289662 52.114.233.161 3479 typ relay raddr 52.112.232.99 rport 50497 MTURNID 5250106049773094678\r\na=candidate:4 1 tcp-pass 1248196606 52.114.231.39 50755 typ relay raddr 52.112.232.99 rport 8129\r\na=label:main-audio\r\na=mid:0\r\na=rtpmap:120 CN/48000\r\na=rtpmap:9 G722/8000\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:13 CN/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-16\r\na=ptime:20\r\na=fingerprint:sha-256 F8:A5:B5:A1:E7:01:F0:63:04:77:64:CC:D1:41:65:C0:36:3A:0F:A2:39:84:34:BF:AF:6C:E2:00:44:F3:F9:2C\r\na=extmap:2 http:\\\\www.webrtc.org\\experiments\\rtp-hdrext\\abs-send-time\r\na=x-ssrc-range:1000-1000\r\nm=video 0 RTP/SAVP 39 40\r\nm=video 0 RTP/SAVP 39 40\r\nm=x-data 0 RTP/SAVP 127 126\r\n";

    #[test]
    fn the_answer_of_teams_as_standard_sdp() {
        let a = from_ngc(ECHO_ANSWER).expect("audio");
        assert_eq!(a.ssrc, Some(1000));
        assert_eq!(a.payload_types, [120, 9, 0, 8, 13, 101]);
        let s = &a.sdp;
        assert!(s.contains("m=audio 50497 UDP/TLS/RTP/SAVPF 120 9 0 8 13 101\r\nc=IN IP4 52.112.232.99\r\n"), "{s}");
        assert!(s.contains("a=setup:passive\r\na=mid:0\r\na=sendrecv\r\na=rtcp-mux\r\n"));
        assert!(s.contains("a=candidate:1 1 udp 2130705918 52.112.232.99 50497 typ srflx raddr 10.128.65.160 rport 50497\r\n"));
        assert!(s.contains("a=candidate:3 1 udp 1258289662 52.114.233.161 3479 typ relay raddr 52.112.232.99 rport 50497\r\n"), "MTURNID dropped");
        assert!(s.contains("a=candidate:2 1 tcp 2121006078 52.112.232.99 50001 typ srflx raddr 10.128.65.160 rport 50001 tcptype active\r\n"));
        assert!(s.contains("a=extmap:2 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time\r\n"));
        assert!(s.contains("a=ssrc:1000 cname:teams-1000\r\n") && !s.contains("m=video"));
        assert!(from_ngc("v=0\r\nm=audio 0 RTP/SAVP 0\r\n").is_none());
    }

    #[test]
    fn a_standard_offer_as_the_offer_of_an_audio_only_teams_client() {
        let offer = "v=0\r\no=- 123 2 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\na=fingerprint:sha-256 AA:BB\r\na=group:BUNDLE 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111 9 0\r\nc=IN IP4 0.0.0.0\r\na=setup:actpass\r\na=mid:0\r\na=ice-ufrag:uf\r\na=ice-pwd:pw\r\na=rtcp-mux\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10;useinbandfec=1\r\na=rtpmap:9 G722/8000\r\na=rtpmap:0 PCMU/8000\r\na=extmap:1 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time\r\na=ssrc:42 cname:x\r\na=candidate:1 1 udp 2130706431 172.17.0.2 40000 typ host\r\na=candidate:2 1 udp 1694498815 217.76.49.98 40000 typ srflx raddr 172.17.0.2 rport 40000\r\na=candidate:3 1 udp 16777215 52.112.172.93 58225 typ relay raddr 217.76.49.98 rport 40000\r\na=candidate:4 1 tcp 1518280447 172.17.0.2 9 typ host tcptype active\r\na=end-of-candidates\r\na=sendrecv\r\n";
        let ngc = to_ngc(offer, &OfferExtras { ssrc: 42, cname: "x".into() }).expect("ngc");
        let l: Vec<&str> = ngc.split("\r\n").collect();
        assert!(l.contains(&"m=audio 58225 RTP/SAVP 111 9 0") && l.contains(&"c=IN IP4 52.112.172.93"), "relay is the address: {ngc}");
        assert!(l.contains(&"a=x-ssrc-range:42-42") && l.contains(&"a=label:main-audio") && l.contains(&"a=rtcp:58225"));
        assert!(l.contains(&"a=fingerprint:sha-256 AA:BB") && l.contains(&"a=ice-ufrag:uf"));
        assert!(l.contains(&"a=extmap:1 http:\\\\www.webrtc.org\\experiments\\rtp-hdrext\\abs-send-time"));
        assert!(l.contains(&"a=candidate:3 1 udp 16777215 52.112.172.93 58225 typ relay raddr 217.76.49.98 rport 40000"));
        assert!(!ngc.contains("tcptype") && l.contains(&"m=video 0 RTP/SAVP 39 40") && l.contains(&"m=x-data 0 RTP/SAVP 127 126"));
        // the answer side of our own offer, as the far end would read it
        let back = from_ngc(&ngc).expect("audio");
        assert_eq!(back.payload_types, [111, 9, 0]);
        assert_eq!(back.ssrc, Some(42));
    }

    /// The offer of Teams desktop on a call to this client (2026-11-15, Vection to Aubay), cut to what the answer reads:
    /// audio_0 and video_1 bundled, no a=setup.
    const DESKTOP_OFFER: &str = "v=0\r\no=- 0 0 IN IP4 52.114.250.1\r\ns=session\r\nc=IN IP4 52.114.250.1\r\nb=CT:4000\r\nt=0 0\r\na=group:BUNDLE audio_0 video_1\r\nm=audio 3480 RTP/SAVP 109 102 9 0 8 101\r\na=x-ssrc-range:46224-46224\r\na=label:main-audio\r\na=ice-ufrag:Ge7I\r\na=ice-pwd:9XmA2vnKbfzRxIqWp0bLtnCs\r\na=candidate:10 1 UDP 1258286591 52.114.250.1 3480 typ relay raddr 151.46.158.178 rport 50450 MTURNID 677281832445125534\r\na=candidate:9 1 UDP 1694494206 151.46.158.178 50450 typ srflx raddr 192.168.8.166 rport 50008\r\na=fingerprint:sha-256 B5:0F:53\r\na=mid:audio_0\r\na=rtcp-mux\r\na=rtpmap:109 SATINFB/48000/2\r\na=rtpmap:102 opus/48000/2\r\na=rtpmap:9 G722/8000\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:101 telephone-event/8000\r\nm=video 3480 RTP/SAVP 119 100\r\nc=IN IP4 52.114.239.83\r\na=x-ssrc-range:24589-24688\r\na=label:main-video\r\na=mid:video_1\r\na=recvonly\r\na=rtcp-mux\r\n";

    #[test]
    fn an_answer_to_teams_desktop_has_the_streams_and_mids_of_its_offer() {
        let offered = from_ngc_offer(DESKTOP_OFFER).expect("audio");
        assert!(offered.sdp.contains("a=setup:actpass\r\n"), "no setup offered: either role");
        assert_eq!(offered.payload_types, [109, 102, 9, 0, 8, 101]);
        let answer = "v=0\r\no=- 7 2 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\na=fingerprint:sha-256 CC:DD\r\na=group:BUNDLE 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 102 9 0 8\r\nc=IN IP4 0.0.0.0\r\na=setup:passive\r\na=mid:0\r\na=ice-ufrag:us\r\na=ice-pwd:pw\r\na=rtcp-mux\r\na=rtpmap:102 opus/48000/2\r\na=rtpmap:9 G722/8000\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\na=candidate:1 1 udp 2130706431 172.25.0.2 34841 typ host\r\na=candidate:2 1 udp 1694498815 217.76.49.98 52747 typ srflx raddr 0.0.0.0 rport 52747\r\na=candidate:3 1 udp 16777215 52.114.251.152 50763 typ relay raddr 0.0.0.0 rport 40000\r\na=sendrecv\r\n";
        let ngc = to_ngc_answer(answer, &OfferExtras { ssrc: 7, cname: "x".into() }, DESKTOP_OFFER).expect("ngc");
        let l: Vec<&str> = ngc.split("\r\n").collect();
        let m: Vec<&&str> = l.iter().filter(|l| l.starts_with("m=")).collect();
        assert_eq!(m, [&"m=audio 50763 RTP/SAVP 102 9 0 8", &"m=video 0 RTP/SAVP 119 100"], "{ngc}");
        assert!(l.contains(&"a=group:BUNDLE audio_0") && l.contains(&"a=mid:audio_0") && l.contains(&"a=mid:video_1"));
        assert!(l.contains(&"a=label:main-video") && l.contains(&"a=setup:passive") && !l.contains(&"a=mid:0"));
        // an offer of Teams web keeps the declined streams of an audio-only Teams web client
        assert!(to_ngc(answer, &OfferExtras { ssrc: 7, cname: "x".into() }).expect("ngc").contains("a=group:BUNDLE 0\r\n"));
    }
}
