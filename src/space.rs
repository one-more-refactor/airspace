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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::{apple_message, microsoft_message, service_meaning, vendor, Observation};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub room: Room,
    pub node: Node,
    pub collector: Collector,
    /// Devices you hold the identity key for, so their rotating addresses can
    /// be recognised as one thing. Written as repeated [[identity]] blocks.
    #[serde(default, rename = "identity")]
    pub identities: Vec<crate::identity::Identity>,
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
            room: Room::default(),
            node: Node::default(),
            collector: Collector::default(),
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

    pub fn load() -> anyhow::Result<Config> {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => Ok(toml::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
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
    pub age: u64,
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
}

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
        let mut i = self.inner.lock().unwrap();
        i.nodes.insert(node.name.clone(), node.clone());
        for o in obs {
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
                    let (metres, calibrated) = match m.map(|o| o.src.as_str()) {
                        // Wi-Fi is a different radio problem and gets its own
                        // constants; sharing them would put every laptop in the
                        // building on the far side of the street.
                        Some("wifi") => (crate::model::wifi_metres(*r), false),
                        _ => crate::model::metres_with_tx(*r, m.and_then(|o| o.tx_power)),
                    };
                    Heard { node: n.clone(), rssi: *r, metres, calibrated, age: now.saturating_sub(*t) }
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
