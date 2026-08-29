//! The Wi-Fi ear.
//!
//! Bluetooth tells you something is present. Wi-Fi tells you rather more: every
//! device associated to an access point announces itself constantly, including
//! laptops, televisions and consoles that have no Bluetooth presence at all.
//!
//! This is the one part of airspace that needs a privilege. A raw packet socket
//! requires `CAP_NET_RAW`, and a monitor interface has to exist already. The
//! Bluetooth half deliberately needs nothing; this half cannot be made to need
//! nothing, and pretending otherwise by quietly asking for root would be worse
//! than saying so.
//!
//! ## What is actually in a frame
//!
//! Measured on an RTL8852BE, which the wikis say cannot do this and which does
//! it fine under the in-kernel `rtw89`:
//!
//!   * the transmitter's MAC, and whether it is randomised — the
//!     locally-administered bit is the same public/private split Bluetooth has;
//!   * signal strength per antenna, from the radiotap header;
//!   * for beacons and probe responses, the network name;
//!   * for probe requests, the network name *when the device names one*.
//!
//! That last point deserves care. The widely repeated claim is that phones
//! broadcast every network they have ever joined, which would make a probe
//! request a travel history. Modern iOS and Android overwhelmingly send
//! wildcard probes with no name in them. Older devices, laptops, IoT things and
//! anything looking for a hidden network still name names — so the leak is real
//! but it is a minority of devices now, not all of them. airspace records what
//! it sees and does not assume.

use std::io;
use std::os::unix::io::RawFd;

use crate::model::Observation;

const ETH_P_ALL: u16 = 0x0003;
const AF_PACKET: libc::c_int = 17;

pub struct Sniffer {
    fd: RawFd,
    buf: Vec<u8>,
}

/// One frame, reduced to the parts that identify something.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Transmitter address. For a client this is the device itself.
    pub source: [u8; 6],
    pub rssi: Option<i8>,
    pub freq: Option<u16>,
    pub kind: Kind,
    /// Network name, when the frame carries one and it is not the empty
    /// wildcard.
    pub ssid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Beacon,
    ProbeRequest,
    ProbeResponse,
    Data,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Beacon => "beacon — this is an access point",
            Kind::ProbeRequest => "probe request — looking for a network",
            Kind::ProbeResponse => "probe response — an access point answering",
            Kind::Data => "carrying traffic — associated and in use",
            Kind::Other => "control or management traffic",
        }
    }
}

impl Sniffer {
    /// Open a raw socket on an existing monitor interface.
    ///
    /// Does not create the interface and does not change its channel: both need
    /// `CAP_NET_ADMIN` on top of `CAP_NET_RAW`, and silently reconfiguring
    /// somebody's radio is not a thing a listening tool should do.
    pub fn open(iface: &str) -> io::Result<Sniffer> {
        let idx = if_index(iface)?;
        let fd = unsafe {
            libc::socket(
                AF_PACKET,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                (ETH_P_ALL as u16).to_be() as libc::c_int,
            )
        };
        if fd < 0 {
            let e = io::Error::last_os_error();
            return Err(io::Error::new(
                e.kind(),
                format!("{e} — a raw socket needs CAP_NET_RAW (see the README)"),
            ));
        }

        let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        addr.sll_family = AF_PACKET as u16;
        addr.sll_protocol = (ETH_P_ALL as u16).to_be();
        addr.sll_ifindex = idx;
        let rc = unsafe {
            libc::bind(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        Ok(Sniffer { fd, buf: vec![0u8; 4096] })
    }

    /// Block for the next frame. `None` means a frame arrived that carries no
    /// identity worth recording, which is most of them.
    pub fn recv(&mut self) -> io::Result<Option<Frame>> {
        let n = unsafe {
            libc::recv(self.fd, self.buf.as_mut_ptr() as *mut libc::c_void, self.buf.len(), 0)
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(e);
        }
        Ok(parse(&self.buf[..n as usize]))
    }
}

impl Drop for Sniffer {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

fn if_index(name: &str) -> io::Result<libc::c_int> {
    let c = std::ffi::CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad interface name"))?;
    let idx = unsafe { libc::if_nametoindex(c.as_ptr()) };
    if idx == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no interface {name:?} — create one with `iw phy phy0 interface add {name} type monitor`"),
        ));
    }
    Ok(idx as libc::c_int)
}

/// Radiotap, then 802.11.
pub fn parse(buf: &[u8]) -> Option<Frame> {
    let (rt_len, rssi, freq) = radiotap(buf)?;
    let f = buf.get(rt_len..)?;
    if f.len() < 24 {
        return None;
    }

    let fc = u16::from_le_bytes([f[0], f[1]]);
    let ftype = (fc >> 2) & 0x3;
    let subtype = (fc >> 4) & 0xF;

    // Address 2 is the transmitter in every frame that has one, which is the
    // device we care about. Control frames often do not have one at all.
    let mut source = [0u8; 6];
    source.copy_from_slice(&f[10..16]);

    let (kind, ie_offset) = match (ftype, subtype) {
        (0, 4) => (Kind::ProbeRequest, 24),
        // Beacons and probe responses carry 12 bytes of fixed parameters —
        // timestamp, interval, capabilities — before the tagged ones.
        (0, 8) => (Kind::Beacon, 36),
        (0, 5) => (Kind::ProbeResponse, 36),
        (2, _) => (Kind::Data, 0),
        (0, _) => (Kind::Other, 0),
        _ => return None,
    };

    let ssid = if ie_offset > 0 { information_element(f, ie_offset, 0) } else { None };
    // An empty SSID is a wildcard probe, not a network called "". Recording it
    // as a name would turn "asked for anything" into "asked for nothing", and
    // both readings are wrong.
    let ssid = ssid.filter(|s| !s.is_empty());

    Some(Frame { source, rssi, freq, kind, ssid })
}

/// Walk the radiotap header far enough to find the channel and the signal.
///
/// Every field is aligned to its own size and they appear in bit order, so the
/// only way to reach field 5 is to step over fields 0 to 4 whether or not
/// anything wants them. Fields above 5 are not walked because nothing here
/// needs them.
fn radiotap(buf: &[u8]) -> Option<(usize, Option<i8>, Option<u16>)> {
    if buf.len() < 8 || buf[0] != 0 {
        return None;
    }
    let len = u16::from_le_bytes([buf[2], buf[3]]) as usize;
    if len < 8 || len > buf.len() {
        return None;
    }

    // The presence bitmap chains: bit 31 set means another word follows.
    let mut present = Vec::new();
    let mut off = 4;
    loop {
        let w = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        present.push(w);
        off += 4;
        if w & (1 << 31) == 0 || off + 4 > len {
            break;
        }
    }

    let first = *present.first()?;
    // (alignment, size) for radiotap fields 0..=5.
    const FIELDS: [(usize, usize); 6] = [(8, 8), (1, 1), (1, 1), (2, 4), (2, 2), (1, 1)];
    let mut rssi = None;
    let mut freq = None;

    for (bit, (align, size)) in FIELDS.iter().enumerate() {
        if first & (1 << bit) == 0 {
            continue;
        }
        off = (off + align - 1) & !(align - 1);
        if off + size > len {
            break;
        }
        match bit {
            3 => freq = Some(u16::from_le_bytes([buf[off], buf[off + 1]])),
            5 => rssi = Some(buf[off] as i8),
            _ => {}
        }
        off += size;
    }
    Some((len, rssi, freq))
}

/// Find one tagged information element and return it as text.
fn information_element(f: &[u8], mut off: usize, want: u8) -> Option<String> {
    while off + 2 <= f.len() {
        let id = f[off];
        let len = f[off + 1] as usize;
        let start = off + 2;
        if start + len > f.len() {
            return None;
        }
        if id == want {
            let raw = &f[start..start + len];
            // Names are arbitrary bytes. Anything unprintable is either a
            // different encoding or a device trying to be clever, and neither
            // belongs in a page rendered as HTML.
            let s: String = String::from_utf8_lossy(raw)
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            return Some(s);
        }
        off = start + len;
    }
    None
}

/// Channel number from the centre frequency. Worth showing because a monitor
/// interface only hears one channel at a time, so "seen on 6" is also a
/// statement about what the listener was capable of hearing at all.
pub fn channel(freq: u16) -> Option<u16> {
    match freq {
        2484 => Some(14),
        2412..=2472 => Some((freq - 2407) / 5),
        5000..=5895 => Some((freq - 5000) / 5),
        _ => None,
    }
}

pub fn mac(a: &[u8; 6]) -> String {
    a.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// Locally-administered bit. Exactly the same distinction Bluetooth draws
/// between a random and a public address, and it is one bit in both.
pub fn is_randomised(a: &[u8; 6]) -> bool {
    a[0] & 0x02 != 0
}

impl Frame {
    pub fn observation(&self, now: u64) -> Observation {
        Observation {
            t: now,
            addr: mac(&self.source),
            at: if is_randomised(&self.source) { "random".into() } else { "public".into() },
            rssi: self.rssi.map(|r| r as i16),
            name: self.ssid.clone(),
            company: Vec::new(),
            cmsg: Vec::new(),
            service: Vec::new(),
            paired: false,
            tx_power: None,
            class: None,
            icon: None,
            modalias: None,
            flags: None,
            in_ear: None,
            src: "wifi".into(),
            doing: Some(match self.freq.and_then(channel) {
                Some(c) => format!("{} (channel {c})", self.kind.as_str()),
                None => self.kind.as_str().to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal radiotap header: version 0, length 13, present = TSFT|Flags|
    /// Rate|Channel|Antenna signal, then those fields with correct alignment.
    fn radiotap_header() -> Vec<u8> {
        let mut v = vec![0u8, 0];
        // bits 1 (flags), 2 (rate), 3 (channel), 5 (signal)
        let present: u32 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 5);
        let len: u16 = 8 + 1 + 1 + 4 + 1;
        v.extend_from_slice(&len.to_le_bytes());
        v.extend_from_slice(&present.to_le_bytes());
        v.push(0x10); // flags
        v.push(2); // rate
        v.extend_from_slice(&2437u16.to_le_bytes()); // channel 6
        v.extend_from_slice(&0u16.to_le_bytes()); // channel flags
        v.push((-45i8) as u8); // signal
        assert_eq!(v.len(), len as usize);
        v
    }

    fn frame_with(fc0: u8, addr2: [u8; 6], tail: &[u8]) -> Vec<u8> {
        let mut v = radiotap_header();
        v.push(fc0);
        v.push(0); // fc byte 2
        v.extend_from_slice(&[0, 0]); // duration
        v.extend_from_slice(&[0xff; 6]); // addr1
        v.extend_from_slice(&addr2); // addr2 — the transmitter
        v.extend_from_slice(&[0xff; 6]); // addr3
        v.extend_from_slice(&[0, 0]); // sequence
        v.extend_from_slice(tail);
        v
    }

    #[test]
    fn reads_signal_and_channel_past_the_alignment() {
        let f = parse(&frame_with(0x40, [0x1e; 6], &[0, 0])).expect("parses");
        assert_eq!(f.rssi, Some(-45));
        assert_eq!(f.freq, Some(2437));
    }

    #[test]
    fn a_named_probe_request_gives_up_the_network() {
        // IE 0, length 4, "home"
        let tail = [0u8, 4, b'h', b'o', b'm', b'e'];
        let f = parse(&frame_with(0x40, [0x1e; 6], &tail)).expect("parses");
        assert_eq!(f.kind, Kind::ProbeRequest);
        assert_eq!(f.ssid.as_deref(), Some("home"));
    }

    #[test]
    fn a_wildcard_probe_is_not_a_network_called_nothing() {
        // IE 0, length 0 — the modern default, and the reason the "phones
        // broadcast their history" claim is mostly out of date.
        let tail = [0u8, 0];
        let f = parse(&frame_with(0x40, [0x1e; 6], &tail)).expect("parses");
        assert_eq!(f.kind, Kind::ProbeRequest);
        assert_eq!(f.ssid, None);
    }

    #[test]
    fn tells_a_randomised_address_from_a_burned_in_one() {
        // 1e:5a:ca:… — locally administered, seen on this desk.
        assert!(is_randomised(&[0x1e, 0x5a, 0xca, 0x03, 0x20, 0x19]));
        // 00:09:52:… — a real OUI, which resolves to a manufacturer.
        assert!(!is_randomised(&[0x00, 0x09, 0x52, 0x09, 0x22, 0x00]));
    }

    #[test]
    fn beacons_skip_the_fixed_parameters() {
        // 12 bytes of fixed params, then IE 0 "UniFi".
        let mut tail = vec![0u8; 12];
        tail.extend_from_slice(&[0, 5, b'U', b'n', b'i', b'F', b'i']);
        let f = parse(&frame_with(0x80, [0x1e; 6], &tail)).expect("parses");
        assert_eq!(f.kind, Kind::Beacon);
        assert_eq!(f.ssid.as_deref(), Some("UniFi"));
    }

    #[test]
    fn converts_frequencies_to_channels() {
        assert_eq!(channel(2437), Some(6));
        assert_eq!(channel(2412), Some(1));
        assert_eq!(channel(5180), Some(36));
        assert_eq!(channel(1234), None);
    }

    #[test]
    fn an_observation_carries_the_channel_it_was_heard_on() {
        let f = parse(&frame_with(0x40, [0x1e; 6], &[0, 0])).expect("parses");
        let o = f.observation(0);
        assert_eq!(o.src, "wifi");
        assert_eq!(o.at, "random");
        assert!(o.doing.unwrap().contains("channel 6"));
    }

    /// A deterministic pseudo-random generator, so a failure is reproducible
    /// rather than "it panicked once on a Tuesday".
    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    #[test]
    fn survives_arbitrary_rubbish() {
        // Every byte this function sees was put on the air by someone else, so
        // "it panics on a malformed frame" is a denial of service that anyone
        // within radio range can trigger. Structured fuzzing is better than
        // this; no fuzzing at all is much worse.
        let mut st = 0x2545F4914F6CDD1Du64;
        for _ in 0..200_000 {
            let len = (xorshift(&mut st) % 300) as usize;
            let mut buf = vec![0u8; len];
            for b in buf.iter_mut() {
                *b = (xorshift(&mut st) & 0xff) as u8;
            }
            let _ = parse(&buf);
        }
    }

    #[test]
    fn survives_plausible_frames_with_lying_lengths() {
        // Random bytes rarely get past the radiotap sanity checks. These are
        // shaped like real frames but with hostile length fields — the case
        // that actually reaches the parsing code.
        let mut st = 0x9E3779B97F4A7C15u64;
        for _ in 0..200_000 {
            let mut buf = radiotap_header();
            buf.push(0x40); // probe request
            buf.push(0);
            buf.extend_from_slice(&[0; 2]);
            buf.extend_from_slice(&[0xff; 18]);
            // A handful of information elements with lengths that overrun,
            // claim zero, or point past the end.
            for _ in 0..(xorshift(&mut st) % 6) {
                buf.push((xorshift(&mut st) & 0xff) as u8);
                buf.push((xorshift(&mut st) & 0xff) as u8);
                buf.push((xorshift(&mut st) & 0xff) as u8);
            }
            // Corrupt the declared radiotap length too.
            if xorshift(&mut st) % 4 == 0 {
                let n = (xorshift(&mut st) & 0xffff) as u16;
                buf[2] = n as u8;
                buf[3] = (n >> 8) as u8;
            }
            let _ = parse(&buf);
        }
    }

    #[test]
    fn refuses_a_truncated_frame() {
        assert!(parse(&radiotap_header()).is_none());
        assert!(parse(&[0u8; 4]).is_none());
    }
}
