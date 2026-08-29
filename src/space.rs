//! Where things are, and how much of that is actually known.
//!
//! One receiver cannot hear a direction. It hears a distance, badly. So the
//! honest picture of a device is not a dot — it is the set of places the device
//! could be, given what every listening node heard.
//!
//! With one node that set is a ring. With two it is a pair of blobs. With three
//! it collapses to a point. The UI draws the set rather than a dot, which means
//! the picture degrades into vagueness instead of into a confident lie, and you
//! can see at a glance how much a reading is worth.
//!
//! This module holds the room, the nodes in it, and the shared live state the
//! collector serves.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::{apple_message, microsoft_message, service_meaning, vendor, Observation};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub room: Room,
    pub node: Node,
    pub collector: Collector,
    /// Fitted path-loss models, one per (node, device). Written by the console
    /// during calibration; absent until something has actually been measured.
    #[serde(default, rename = "calibration")]
    pub calibrations: Vec<crate::model::Calibration>,
    /// Devices you hold the identity key for, so their rotating addresses can
    /// be recognised as one thing. Written as repeated [[identity]] blocks.
    #[serde(default, rename = "identity")]
    pub identities: Vec<crate::identity::Identity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    /// Report only devices matching a configured identity.
    ///
    /// This is the difference between a presence system and a log of the
    /// building. With it on, a stranger's device is dropped the moment it is
    /// recognised as a stranger's, and never reaches state, the page or the
    /// disk — so there is nothing to delete later and no policy to trust.
    ///
    /// Defaults to true, and does nothing at all until at least one identity
    /// is configured: a fresh install with no identities would otherwise show
    /// an empty screen and look broken.
    pub track_only_known: bool,
    /// Treat an earbud's readings as meaningful only while it is being worn.
    ///
    /// In an ear, in a pocket, on a desk and in a case are four different radio
    /// situations, and averaging across them produces a distance that is wrong
    /// in all four.
    pub require_worn: bool,
    /// How long an in-ear reading stays valid — long enough to take a bud out
    /// for a conversation without the device vanishing from the map.
    pub worn_memory_secs: u64,
}

impl Default for General {
    fn default() -> Self {
        General { track_only_known: true, require_worn: true, worn_memory_secs: 600 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Room {
    /// Metres. Only used to draw the picture and to bound the search grid.
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Node {
    pub name: String,
    /// Metres from the top-left corner of the room.
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Collector {
    /// Where `airspace feed` sends its observations.
    pub url: String,
    /// Shared secret. An open ingest endpoint lets anyone draw imaginary
    /// people into your room, which is a worse failure than not having one.
    pub token: String,
}

impl Default for Room {
    fn default() -> Self {
        Room { width: 10.0, height: 8.0 }
    }
}
impl Default for Node {
    fn default() -> Self {
        // The middle of the default room. A node in a corner is the honest
        // position for most desks, but as a first-run default it pushes every
        // ring off the edge of the picture and the tool looks broken before it
        // has said anything. Put your real position in the config.
        Node { name: hostname(), x: 5.0, y: 4.0 }
    }
}
impl Default for Collector {
    fn default() -> Self {
        Collector { url: String::new(), token: String::new() }
    }
}
impl Default for Config {
    fn default() -> Self {
        Config {
            general: General::default(),
            room: Room::default(),
            node: Node::default(),
            collector: Collector::default(),
            calibrations: Vec::new(),
            identities: Vec::new(),
        }
    }
}

impl Config {
    pub fn path() -> std::path::PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join(".config")
            });
        base.join("airspace/config.toml")
    }

    /// Where measured calibrations live.
    ///
    /// A separate file from the config on purpose. The console writes these and
    /// the daemon only reads them, so the daemon never needs write access to
    /// your home directory — its unit sets ProtectHome=read-only and that
    /// stays true. It also means a rewritten calibration cannot clobber the
    /// comments in a config file a human maintains.
    pub fn calibration_path() -> std::path::PathBuf {
        Self::path().with_file_name("calibration.toml")
    }

    pub fn load() -> anyhow::Result<Config> {
        let mut cfg: Config = match std::fs::read_to_string(Self::path()) {
            Ok(s) => toml::from_str(&s)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(e.into()),
        };
        if let Ok(s) = std::fs::read_to_string(Self::calibration_path()) {
            #[derive(serde::Deserialize)]
            struct Cals {
                #[serde(default, rename = "calibration")]
                calibrations: Vec<crate::model::Calibration>,
            }
            let c: Cals = toml::from_str(&s)?;
            // Measured beats configured: a later entry for the same pair wins,
            // so recalibrating is just writing the file again.
            for cal in c.calibrations {
                cfg.calibrations.retain(|x| !(x.node == cal.node && x.device == cal.device));
                cfg.calibrations.push(cal);
            }
        }
        Ok(cfg)
    }
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "here".into())
}

/// What one node heard from one device, most recently.
#[derive(Debug, Clone, Serialize)]
pub struct Heard {
    pub node: String,
    pub rssi: i16,
    /// Metres, from the path-loss model. Wrong, but wrong in a documented way.
    pub metres: f32,
    /// True when the distance used the power the device advertises rather than
    /// an assumed one. The UI says which, because they are not equally good.
    pub calibrated: bool,
    /// How the distance was arrived at — measured here, from an advertised
    /// transmit power, or from textbook constants.
    pub basis: crate::model::Basis,
    pub age: u64,
    /// Fraction of this node's recent sweeps in which the device appeared.
    ///
    /// Deliberately not called a packet rate. The collector sees sweeps, not
    /// individual advertisements, so what it can honestly measure is how often
    /// a device shows up when the node looks — which is the number that says
    /// "this node is badly placed" and it says it clearly.
    pub heard_ratio: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Live {
    pub id: String,
    pub label: String,
    pub at: String,
    pub vendor: Option<String>,
    pub doing: Vec<String>,
    pub leaks: Vec<String>,
    pub heard: Vec<Heard>,
    pub first_seen: u64,
    pub paired: bool,
    /// True when this device matched a configured identity.
    pub known: bool,
    /// Last reported in-ear state, and whether it counts as worn right now.
    pub in_ear: Option<bool>,
    pub worn: bool,
    /// Set when the readings are being shown but should not be trusted for
    /// position — an earbud out of an ear is the case that matters.
    pub unreliable: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub room: Room,
    pub nodes: Vec<Node>,
    pub devices: Vec<Live>,
    pub now: u64,
    /// True when the picture can, in principle, contain a direction.
    pub can_locate: bool,
}

#[derive(Default)]
struct Inner {
    /// addr -> node -> (rssi, when)
    heard: HashMap<String, HashMap<String, (i16, u64)>>,
    meta: HashMap<String, Observation>,
    first: HashMap<String, u64>,
    nodes: HashMap<String, Node>,
    /// When each node last delivered a sweep, most recent last. The
    /// denominator of the heard ratio.
    sweeps: HashMap<String, VecDeque<u64>>,
    /// When each (address, node) pair was present in one of those sweeps.
    seen_in: HashMap<(String, String), VecDeque<u64>>,
    /// When each address was last reported as being in an ear.
    worn_at: HashMap<String, u64>,
}

/// The window over which the heard ratio is computed. Long enough that a
/// single missed sweep is not alarming, short enough to notice a node that has
/// just been moved behind a fridge.
const RATIO_WINDOW: u64 = 60;

#[derive(Clone)]
pub struct State {
    inner: Arc<Mutex<Inner>>,
    pub config: Config,
}

/// A device unheard for this long has left, or rotated its address. Either way
/// it is not in the room any more and drawing it there is a lie.
const STALE: u64 = 45;

impl State {
    pub fn new(config: Config) -> State {
        let mut inner = Inner::default();
        inner.nodes.insert(config.node.name.clone(), config.node.clone());
        State { inner: Arc::new(Mutex::new(inner)), config }
    }

    pub fn ingest(&self, node: &Node, obs: &[Observation]) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut i = self.inner.lock().unwrap();
        i.nodes.insert(node.name.clone(), node.clone());

        // One sweep from this node, whatever it contained. Recorded before the
        // observations so that a node reporting nothing still counts as having
        // looked — "heard nothing" and "stopped reporting" are different
        // failures and must not look alike.
        let sweeps = i.sweeps.entry(node.name.clone()).or_default();
        sweeps.push_back(now);
        while sweeps.front().is_some_and(|t| now.saturating_sub(*t) > RATIO_WINDOW) {
            sweeps.pop_front();
        }

        for o in obs {
            // Drop a stranger here, before it reaches state, the page or the
            // disk. Filtering later would still mean having written it down.
            if self.config.general.track_only_known
                && !self.config.identities.is_empty()
                && crate::identity::whose(&self.config.identities, &o.addr).is_none()
            {
                continue;
            }
            let key = (o.addr.clone(), node.name.clone());
            let seen = i.seen_in.entry(key).or_default();
            seen.push_back(now);
            while seen.front().is_some_and(|t| now.saturating_sub(*t) > RATIO_WINDOW) {
                seen.pop_front();
            }
            if o.in_ear == Some(true) {
                i.worn_at.insert(o.addr.clone(), now);
            }
            let Some(rssi) = o.rssi else { continue };
            i.heard
                .entry(o.addr.clone())
                .or_default()
                .insert(node.name.clone(), (rssi, o.t));
            i.first.entry(o.addr.clone()).or_insert(o.t);
            let e = i.meta.entry(o.addr.clone()).or_insert_with(|| o.clone());
            // Keep the most informative version: a name or a payload seen once
            // is still true after an advertisement that omitted it.
            if e.name.is_none() {
                e.name = o.name.clone();
            }
            for c in &o.company {
                if !e.company.contains(c) {
                    e.company.push(*c);
                }
            }
            for m in &o.cmsg {
                if !e.cmsg.contains(m) {
                    e.cmsg.push(*m);
                }
            }
            for s in &o.service {
                if !e.service.contains(s) {
                    e.service.push(s.clone());
                }
            }
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let i = self.inner.lock().unwrap();
        let mut devices = Vec::new();

        for (addr, per_node) in &i.heard {
            let heard: Vec<Heard> = per_node
                .iter()
                .filter(|(_, (_, t))| now.saturating_sub(*t) <= STALE)
                .map(|(n, (r, t))| {
                    let m = i.meta.get(addr);
                    let who = crate::identity::whose(&self.config.identities, addr);
                    // A model fitted in this room for this pair beats one
                    // derived from an advertised power, which beats a textbook
                    // constant. Take the best available and say which it was.
                    let cal = who.as_ref().and_then(|w| {
                        self.config
                            .calibrations
                            .iter()
                            .find(|c| &c.node == n && &c.device == w)
                    });
                    let (metres, calibrated, basis) = match (cal, m.map(|o| o.src.as_str())) {
                        (Some(c), _) => (c.metres(*r), true, crate::model::Basis::Measured),
                        (None, Some("wifi")) => {
                            (crate::model::wifi_metres(*r), false, crate::model::Basis::Assumed)
                        }
                        (None, _) => {
                            let (d, c) = crate::model::metres_with_tx(*r, m.and_then(|o| o.tx_power));
                            let b = if c {
                                crate::model::Basis::Advertised
                            } else {
                                crate::model::Basis::Assumed
                            };
                            (d, c, b)
                        }
                    };
                    let sweeps = i.sweeps.get(n).map(|v| v.len()).unwrap_or(0);
                    let hits = i
                        .seen_in
                        .get(&(addr.clone(), n.clone()))
                        .map(|v| v.len())
                        .unwrap_or(0);
                    let heard_ratio =
                        if sweeps == 0 { 0.0 } else { (hits as f32 / sweeps as f32).min(1.0) };
                    Heard {
                        node: n.clone(),
                        rssi: *r,
                        metres,
                        calibrated,
                        basis,
                        age: now.saturating_sub(*t),
                        heard_ratio,
                    }
                })
                .collect();
            if heard.is_empty() {
                continue;
            }
            let Some(o) = i.meta.get(addr) else { continue };

            let mut doing = Vec::new();
            if let Some(d) = &o.doing {
                doing.push(d.clone());
            }
            for (c, m) in &o.cmsg {
                let d = match c {
                    0x004C => apple_message(*m),
                    0x0006 => microsoft_message(*m),
                    _ => None,
                };
                if let Some(d) = d {
                    doing.push(d.to_string());
                }
            }
            // Everything that narrows down WHAT the thing is, best evidence
            // first: an exact model beats a category beats a guess.
            let mut kind: Vec<String> = Vec::new();
            if let Some(m) = &o.modalias {
                kind.push(format!("exact model, from its modalias: {m}"));
            }
            if let Some(c) = o.class.and_then(crate::model::device_class) {
                kind.push(format!("device class says: {c}"));
            }
            if let Some(ic) = &o.icon {
                kind.push(format!("BlueZ classifies it as: {ic}"));
            }
            if let Some(f) = o.flags {
                kind.push(crate::model::radio_kind(f).to_string());
            }
            if o.service.iter().any(|s| s.get(4..8).is_some_and(|x| x.eq_ignore_ascii_case("1812"))) {
                kind.push("advertises HID — a keyboard, mouse or controller".into());
            }

            let mut leaks: Vec<String> = o
                .service
                .iter()
                .filter_map(|s| service_meaning(s))
                .map(str::to_string)
                .collect();
            if o.at == "public" {
                leaks.push("permanent hardware address — this identifier never rotates".into());
            }

            let v = o.company.iter().find_map(|c| vendor(*c)).map(str::to_string);
            leaks.extend(kind);

            // A device we hold the key for is not "unnamed Apple device" — it
            // is a thing with a name, however many times it has changed its
            // address since we last looked.
            let known = crate::identity::whose(&self.config.identities, addr);

            // Worn state, with a memory: a bud taken out for a conversation
            // should not make the device disappear from the map.
            let worn = i
                .worn_at
                .get(addr)
                .is_some_and(|t| now.saturating_sub(*t) < self.config.general.worn_memory_secs);
            let reports_ear = o.in_ear.is_some();
            let unreliable = if self.config.general.require_worn && reports_ear && !worn {
                Some(
                    "not being worn — an earbud on a desk or in a pocket is a different \
                     radio problem, so this distance is not comparable"
                        .to_string(),
                )
            } else {
                None
            };
            if known.is_some() {
                leaks.push(
                    "address resolved with its identity key — rotation does not hide this device \
                     from anyone holding that key"
                        .into(),
                );
            }
            devices.push(Live {
                id: addr.clone(),
                label: known.clone().or_else(|| o.name.clone()).unwrap_or_else(|| match &v {
                    Some(v) => format!("unnamed {v} device"),
                    None => "unnamed device".into(),
                }),
                at: o.at.clone(),
                vendor: v,
                doing,
                leaks,
                heard,
                first_seen: i.first.get(addr).copied().unwrap_or(now),
                paired: o.paired,
                known: known.is_some(),
                in_ear: o.in_ear,
                worn,
                unreliable,
            });
        }

        devices.sort_by(|a, b| {
            let am = a.heard.iter().map(|h| h.metres).fold(f32::MAX, f32::min);
            let bm = b.heard.iter().map(|h| h.metres).fold(f32::MAX, f32::min);
            am.partial_cmp(&bm).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut nodes: Vec<Node> = i.nodes.values().cloned().collect();
        nodes.sort_by(|a, b| a.name.cmp(&b.name));

        Snapshot {
            room: self.config.room.clone(),
            can_locate: nodes.len() >= 3,
            nodes,
            devices,
            now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(addr: &str, rssi: i16, t: u64) -> Observation {
        Observation {
            t,
            addr: addr.into(),
            at: "random".into(),
            rssi: Some(rssi),
            name: None,
            company: vec![0x004C],
            cmsg: vec![(0x004C, 0x10)],
            service: vec![],
            paired: false,
            tx_power: None,
            class: None,
            icon: None,
            modalias: None,
            flags: None,
            src: "ble".into(),
            doing: None,
            in_ear: None,
        }
    }

    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    }

    #[test]
    fn one_node_cannot_locate() {
        let s = State::new(Config::default());
        s.ingest(&Node { name: "a".into(), x: 0.0, y: 0.0 }, &[obs("AA", -60, now())]);
        let snap = s.snapshot();
        assert!(!snap.can_locate, "a single receiver has no bearing to give");
        assert_eq!(snap.devices.len(), 1);
        assert_eq!(snap.devices[0].heard.len(), 1);
    }

    #[test]
    fn three_nodes_can() {
        let s = State::new(Config::default());
        for (n, r) in [("a", -60), ("b", -70), ("c", -80)] {
            s.ingest(&Node { name: n.into(), x: 1.0, y: 1.0 }, &[obs("AA", r, now())]);
        }
        let snap = s.snapshot();
        assert!(snap.can_locate);
        assert_eq!(snap.devices[0].heard.len(), 3);
    }

    #[test]
    fn stale_devices_leave_the_picture() {
        let s = State::new(Config::default());
        s.ingest(&Node::default(), &[obs("AA", -60, now() - STALE - 5)]);
        assert!(s.snapshot().devices.is_empty());
    }
}
