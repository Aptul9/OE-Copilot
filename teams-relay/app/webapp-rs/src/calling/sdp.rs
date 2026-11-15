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

/// The standard offer or answer of the peer connection (ICE gathered) in the dialect, as an audio-only Teams client
/// sends it. None when it has no audio or no candidate.
pub fn to_ngc(offer: &str, extras: &OfferExtras) -> Option<String> {
    let (session, media) = sections(offer);
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
        "a=group:BUNDLE 0".into(),
        format!("m=audio {} RTP/SAVP {}", at.port, pts.join(" ")),
        format!("c=IN IP4 {}", at.ip),
        "a=x-signaling-fb:* x-message app recv:dsh".into(),
        format!("a=x-ssrc-range:{0}-{0}", extras.ssrc),
    ];
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
    o.push("a=mid:0".into());
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
    // the streams an audio-only client declines
    for (m, label) in
        [("m=video 0 RTP/SAVP 39 40", "main-video"), ("m=video 0 RTP/SAVP 39 40", "applicationsharing-video"), ("m=x-data 0 RTP/SAVP 127 126", "data")]
    {
        o.push(m.into());
        o.push(format!("a=label:{label}"));
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

/// A dialect answer (or offer) as the standard SDP of an audio-only peer connection whose audio has mid "0". None when
/// it accepts no audio.
pub fn from_ngc(ngc: &str) -> Option<Answered> {
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
    o.push(format!("a=setup:{}", from("setup").unwrap_or("active")));
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
}
