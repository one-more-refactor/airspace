//! Turning a pile of sightings into the two questions worth asking:
//! how many people are near you, and what did their pockets give away.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::model::{apple_message, microsoft_message, service_meaning, vendor, Observation};

/// Everything one address did while it existed.
#[derive(Debug, Clone)]
pub struct Track {
    pub addr: String,
    pub at: String,
    pub first: u64,
    pub last: u64,
    pub sightings: usize,
    pub best_rssi: Option<i16>,
    pub median_rssi: Option<i16>,
    pub name: Option<String>,
    pub company: BTreeSet<u16>,
    pub cmsg: BTreeSet<(u16, u8)>,
    pub service: BTreeSet<String>,
    pub paired: bool,
}

impl Track {
    pub fn lifetime(&self) -> u64 {
        self.last.saturating_sub(self.first)
    }

    /// What this address is, in the sense a human would answer it.
    pub fn label(&self) -> String {
        if let Some(n) = &self.name {
            return n.clone();
        }
        let v: Vec<&str> = self.company.iter().filter_map(|c| vendor(*c)).collect();
        match v.first() {
            Some(v) => format!("unnamed {v} device"),
            None => "unnamed device".to_string(),
        }
    }

    /// The plain-language list of what this device announced about itself.
    pub fn leaks(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.at == "public" {
            out.push(
                "a permanent hardware address — this identifier does not rotate, ever".to_string(),
            );
        }
        if let Some(n) = &self.name {
            out.push(format!("a name it chose to broadcast: {n:?}"));
        }
        for c in &self.company {
            if let Some(v) = vendor(*c) {
                out.push(format!("who made it: {v}"));
            }
        }
        for (company, m) in &self.cmsg {
            let known = match company {
                0x004C => apple_message(*m),
                0x0006 => microsoft_message(*m),
                _ => None,
            };
            if let Some(k) = known {
                out.push(format!("what it is doing: {k}"));
            }
        }
        for s in &self.service {
            if let Some(k) = service_meaning(s) {
                out.push(format!("a service it advertises: {k}"));
            }
        }
        out
    }

    /// The shape of a device independent of its current address. Two addresses
    /// with the same fingerprint are the same KIND of thing; whether they are
    /// the same thing is what `chains` tries to decide.
    fn fingerprint(&self) -> String {
        let c: Vec<String> = self.company.iter().map(|x| format!("{x:04x}")).collect();
        let m: Vec<String> = self.cmsg.iter().map(|(c, x)| format!("{c:04x}:{x:02x}")).collect();
        let s: Vec<String> = self.service.iter().map(|x| x[4..8].to_lowercase()).collect();
        format!("{}|{}|{}", c.join(","), m.join(","), s.join(","))
    }
}

pub struct Analysis {
    pub tracks: Vec<Track>,
    pub chains: Vec<Vec<String>>,
    pub window: (u64, u64),
    pub sightings: usize,
}

impl Analysis {
    pub fn public_addrs(&self) -> usize {
        self.tracks.iter().filter(|t| t.at == "public").count()
    }
    pub fn random_addrs(&self) -> usize {
        self.tracks.iter().filter(|t| t.at == "random").count()
    }
    /// Addresses, minus the ones a chain says were the same device wearing a
    /// different number. The honest count of things in the room is between
    /// this and `tracks.len()`.
    pub fn estimated_devices(&self) -> usize {
        let chained: usize = self.chains.iter().map(|c| c.len()).sum();
        self.tracks.len() - chained + self.chains.len()
    }
    pub fn duration(&self) -> u64 {
        self.window.1.saturating_sub(self.window.0)
    }
    /// An address still being heard when the capture stopped tells you nothing
    /// about its lifetime except that it was longer than the window. Counting
    /// those as observed lifetimes drags the median toward the window length
    /// and makes randomization look better the shorter you listen.
    pub fn censored(&self) -> usize {
        self.tracks.iter().filter(|t| t.last + 10 >= self.window.1).count()
    }

    /// Median lifetime of the addresses that actually stopped, censored ones
    /// excluded.
    ///
    /// It is NOT a rotation interval. An address going quiet means either the
    /// device rotated its identity or the device left the building, and a
    /// single receiver cannot tell those apart. It is an upper bound on how
    /// long one identifier stayed usable.
    pub fn median_lifetime(&self) -> Option<u64> {
        let mut v: Vec<u64> = self
            .tracks
            .iter()
            .filter(|t| t.at == "random" && t.sightings > 1 && t.last + 10 < self.window.1)
            .map(|t| t.lifetime())
            .collect();
        if v.is_empty() {
            return None;
        }
        v.sort_unstable();
        Some(v[v.len() / 2])
    }
}

pub fn analyse(obs: &[Observation]) -> Analysis {
    let mut by_addr: BTreeMap<String, Track> = BTreeMap::new();
    let mut rssis: HashMap<String, Vec<i16>> = HashMap::new();

    for o in obs {
        let t = by_addr.entry(o.addr.clone()).or_insert_with(|| Track {
            addr: o.addr.clone(),
            at: o.at.clone(),
            first: o.t,
            last: o.t,
            sightings: 0,
            best_rssi: None,
            median_rssi: None,
            name: None,
            company: BTreeSet::new(),
            cmsg: BTreeSet::new(),
            service: BTreeSet::new(),
            paired: false,
        });
        t.first = t.first.min(o.t);
        t.last = t.last.max(o.t);
        t.sightings += 1;
        t.paired |= o.paired;
        if t.name.is_none() {
            t.name = o.name.clone();
        }
        t.company.extend(o.company.iter().copied());
        t.cmsg.extend(o.cmsg.iter().copied());
        t.service.extend(o.service.iter().cloned());
        if let Some(r) = o.rssi {
            t.best_rssi = Some(t.best_rssi.map_or(r, |b| b.max(r)));
            rssis.entry(o.addr.clone()).or_default().push(r);
        }
    }

    for (addr, mut v) in rssis {
        v.sort_unstable();
        if let Some(t) = by_addr.get_mut(&addr) {
            t.median_rssi = Some(v[v.len() / 2]);
        }
    }

    let tracks: Vec<Track> = by_addr.into_values().collect();
    let chains = chains(&tracks);
    let window = (
        obs.iter().map(|o| o.t).min().unwrap_or(0),
        obs.iter().map(|o| o.t).max().unwrap_or(0),
    );

    Analysis { tracks, chains, window, sightings: obs.len() }
}

/// Link addresses that are probably one device rotating its identity.
///
/// The rule is deliberately conservative and entirely explainable, because the
/// alternative — a classifier — produces a number nobody can check:
///
///   * same advertising fingerprint (vendor, message types, services), and
///   * lifetimes that do not overlap, and
///   * the next address appears within 30s of the previous one going quiet.
///
/// Published de-randomization work does much better than this using frame
/// timing, sequence numbers and payload field ordering, none of which bluez
/// exposes. So this is a floor on what is linkable, not a ceiling — which is
/// the useful direction for a tool meant to show you what leaks.
const HANDOVER: u64 = 30;

fn chains(tracks: &[Track]) -> Vec<Vec<String>> {
    let mut groups: HashMap<String, Vec<&Track>> = HashMap::new();
    for t in tracks.iter().filter(|t| t.at == "random") {
        let fp = t.fingerprint();
        // An empty fingerprint says nothing about anything; refusing to chain
        // on it is the difference between a heuristic and a random number.
        if fp == "||" {
            continue;
        }
        groups.entry(fp).or_default().push(t);
    }

    let mut out = Vec::new();
    for (_fp, mut g) in groups {
        if g.len() < 2 {
            continue;
        }
        g.sort_by_key(|t| t.first);
        let mut chain: Vec<String> = vec![g[0].addr.clone()];
        let mut prev_end = g[0].last;
        for t in &g[1..] {
            if t.first >= prev_end && t.first - prev_end <= HANDOVER {
                chain.push(t.addr.clone());
                prev_end = t.last;
            } else if chain.len() > 1 {
                out.push(std::mem::take(&mut chain));
                chain = vec![t.addr.clone()];
                prev_end = t.last;
            } else {
                chain = vec![t.addr.clone()];
                prev_end = t.last;
            }
        }
        if chain.len() > 1 {
            out.push(chain);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(t: u64, addr: &str, at: &str, rssi: i16, company: Vec<u16>) -> Observation {
        Observation {
            t,
            addr: addr.into(),
            at: at.into(),
            rssi: Some(rssi),
            name: None,
            company: company.clone(),
            cmsg: company.iter().map(|c| (*c, 0x10u8)).collect(),
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

    #[test]
    fn folds_sightings_into_tracks() {
        let a = analyse(&[
            obs(100, "AA:BB", "public", -60, vec![0x004C]),
            obs(102, "AA:BB", "public", -70, vec![0x004C]),
        ]);
        assert_eq!(a.tracks.len(), 1);
        assert_eq!(a.tracks[0].sightings, 2);
        assert_eq!(a.tracks[0].best_rssi, Some(-60));
        assert_eq!(a.tracks[0].lifetime(), 2);
        assert_eq!(a.public_addrs(), 1);
    }

    #[test]
    fn chains_a_rotation_but_not_a_coincidence() {
        // Two addresses, same fingerprint, one starts 10s after the other ends.
        let a = analyse(&[
            obs(100, "11:11", "random", -60, vec![0x004C]),
            obs(120, "11:11", "random", -61, vec![0x004C]),
            obs(130, "22:22", "random", -60, vec![0x004C]),
            obs(150, "22:22", "random", -62, vec![0x004C]),
        ]);
        assert_eq!(a.chains.len(), 1, "the rotation should link");
        assert_eq!(a.estimated_devices(), 1);

        // Same fingerprint but overlapping in time: two different devices.
        let b = analyse(&[
            obs(100, "11:11", "random", -60, vec![0x004C]),
            obs(150, "11:11", "random", -61, vec![0x004C]),
            obs(120, "22:22", "random", -60, vec![0x004C]),
            obs(160, "22:22", "random", -62, vec![0x004C]),
        ]);
        assert!(b.chains.is_empty(), "overlapping lifetimes cannot be one device");
        assert_eq!(b.estimated_devices(), 2);
    }

    #[test]
    fn refuses_to_chain_on_an_empty_fingerprint() {
        let a = analyse(&[
            Observation { cmsg: vec![], ..obs(100, "11:11", "random", -60, vec![]) },
            Observation { cmsg: vec![], ..obs(130, "22:22", "random", -60, vec![]) },
        ]);
        assert!(a.chains.is_empty());
    }
}
