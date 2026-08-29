//! Resolving rotating addresses back to a device you own.
//!
//! A phone does not advertise under a fixed address. It uses a Resolvable
//! Private Address, which rotates every fifteen minutes or so — that is why a
//! capture of a flat is mostly a list of `random` Apple devices that appear,
//! live for a few minutes and are never seen again.
//!
//! An RPA is not noise, though. It is `prand || hash`, where the bottom three
//! bytes are a keyed hash of the top three under the device's Identity
//! Resolving Key. Hold that key and any address the device emits can be tested
//! and recognised. This is what ESPresense does to follow an iPhone around a
//! house, and it is the only honest answer to "my phone is invisible to my own
//! tools".
//!
//! ## The line this draws
//!
//! You get the IRK by *bonding with the device* — it is handed over during
//! pairing, and BlueZ keeps it in `/var/lib/bluetooth/<adapter>/<device>/info`.
//! So this resolves devices you own and have paired. It does nothing whatsoever
//! for the strangers in a capture, and it cannot: their keys were never shared
//! with you.
//!
//! That is the difference between a presence system and surveillance, and it is
//! a real one rather than a policy one. The maths simply does not work without
//! the key.
//!
//! An IRK is also a serious secret. Anyone holding it can recognise that phone
//! anywhere, forever, regardless of how diligently it rotates its address. It
//! belongs in a `0600` config file and nowhere else.

use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes128;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Identity {
    /// What to call it on the map.
    pub name: String,
    /// 32 hex characters, from the `[IdentityResolvingKey]` block of the
    /// device's BlueZ bond file. Needed on any node that is NOT bonded to the
    /// device — see the note on `address`.
    pub irk: String,
    /// A fixed address to match outright.
    ///
    /// This is not redundant with the IRK, and the difference cost an evening
    /// to find. Once a device is bonded, BlueZ loads its IRK into the
    /// controller's resolving list and the hardware translates every private
    /// address before userspace ever sees it — so on the bonded machine the
    /// device shows up under its permanent identity address and there is
    /// nothing left to resolve. On an unbonded node, the same device arrives
    /// as a rotating private address and only the IRK will do.
    ///
    /// So: `address` labels it on the machine you paired with, `irk` labels it
    /// everywhere else, and a device you care about wants both.
    pub address: String,
}

impl Default for Identity {
    fn default() -> Self {
        Identity { name: String::new(), irk: String::new(), address: String::new() }
    }
}

impl Identity {
    pub fn key(&self) -> Option<[u8; 16]> {
        parse_hex16(&self.irk)
    }
}

pub fn parse_hex16(s: &str) -> Option<[u8; 16]> {
    let s: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// The `ah` function from the Core Specification, Vol 3 Part H.
///
/// `ah(k, r) = e(k, padding || r)` truncated to its least significant three
/// bytes, where `padding` is thirteen zero octets. There is a published test
/// vector for this and it is exercised below, because getting the byte order
/// wrong here produces something that never matches anything and looks exactly
/// like a device that simply is not there.
pub fn ah(irk: &[u8; 16], prand: &[u8; 3]) -> [u8; 3] {
    let cipher = Aes128::new(irk.into());
    let mut block = [0u8; 16];
    block[13] = prand[0];
    block[14] = prand[1];
    block[15] = prand[2];
    let mut b = aes::Block::from(block);
    cipher.encrypt_block(&mut b);
    [b[13], b[14], b[15]]
}

/// Is this address one that key generated?
///
/// `addr` is in printed order — the leftmost octet of `AA:BB:CC:DD:EE:FF`
/// first, which is how BlueZ shows it and how airspace stores it.
pub fn resolves(irk: &[u8; 16], addr: &[u8; 6]) -> bool {
    // Only resolvable private addresses can resolve. The top two bits of the
    // most significant octet are 0b01 for an RPA; anything else is a public
    // address, a static random one, or a non-resolvable private one, and
    // running the maths on those would produce false matches at a rate of one
    // in sixteen million per key — rare enough to look like a real sighting.
    if addr[0] & 0xc0 != 0x40 {
        return false;
    }
    let prand = [addr[0], addr[1], addr[2]];
    let hash = [addr[3], addr[4], addr[5]];
    ah(irk, &prand) == hash
}

/// Parse "AA:BB:CC:DD:EE:FF" into bytes in printed order.
pub fn parse_addr(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut out = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        out[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(out)
}

/// Which identity, if any, does this address belong to?
pub fn whose(identities: &[Identity], addr: &str) -> Option<String> {
    for id in identities {
        if !id.address.is_empty() && id.address.eq_ignore_ascii_case(addr) {
            return Some(id.name.clone());
        }
    }
    let bytes = parse_addr(addr)?;
    for id in identities {
        if let Some(k) = id.key() {
            if resolves(&k, &bytes) {
                return Some(id.name.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_test_vector() {
        // Core Specification, Vol 3, Part H, Appendix D.7 — the worked example
        // for `ah`. If this passes, the crypto and the byte order are right,
        // and any failure to resolve a real device is a key problem rather
        // than an implementation one. That distinction is worth a test.
        let irk = parse_hex16("ec0234a357c8ad05341010a60a397d9b").unwrap();
        let prand = [0x70u8, 0x81, 0x94];
        assert_eq!(ah(&irk, &prand), [0x0d, 0xfb, 0xaa]);
    }

    #[test]
    fn resolves_an_address_built_from_the_key() {
        let irk = parse_hex16("ec0234a357c8ad05341010a60a397d9b").unwrap();
        // Construct a valid RPA: prand with the top bits set to 0b01, then the
        // hash the key produces for it.
        let prand = [0x70u8, 0x81, 0x94];
        assert_eq!(prand[0] & 0xc0, 0x40, "test prand must look like an RPA");
        let h = ah(&irk, &prand);
        let addr = [prand[0], prand[1], prand[2], h[0], h[1], h[2]];
        assert!(resolves(&irk, &addr));

        // A different key must not claim it.
        let other = parse_hex16("00112233445566778899aabbccddeeff").unwrap();
        assert!(!resolves(&other, &addr));
    }

    #[test]
    fn refuses_addresses_that_are_not_resolvable_private() {
        let irk = parse_hex16("ec0234a357c8ad05341010a60a397d9b").unwrap();
        // Public address — top bits 0b00. Testing it would be meaningless and
        // occasionally produce a false positive.
        assert!(!resolves(&irk, &[0x1c, 0xb3, 0xc9, 0xc5, 0x44, 0x14]));
        // Static random — top bits 0b11.
        assert!(!resolves(&irk, &[0xd0, 0x00, 0x00, 0x00, 0x00, 0x00]));
    }

    #[test]
    fn a_fixed_address_matches_without_any_key() {
        // The bonded-machine case: the controller already resolved it, so what
        // arrives is the permanent address and there is no key work to do.
        let ids = vec![Identity {
            name: "phone".into(),
            irk: String::new(),
            address: "EC:A9:07:A0:3A:60".into(),
        }];
        assert_eq!(whose(&ids, "EC:A9:07:A0:3A:60").as_deref(), Some("phone"));
        assert_eq!(whose(&ids, "ec:a9:07:a0:3a:60").as_deref(), Some("phone"));
        assert_eq!(whose(&ids, "11:22:33:44:55:66"), None);
    }

    #[test]
    fn address_and_key_parsing_never_panics() {
        // These take strings from a config file a human edits, which is its own
        // kind of hostile input.
        for s in [
            "", ":", "::::::", "ZZ:ZZ:ZZ:ZZ:ZZ:ZZ", "1:2:3:4:5:6",
            "EC:A9:07:A0:3A:60:00", "-1:00:00:00:00:00",
            &"f".repeat(1000), &"0".repeat(31), &"0".repeat(33),
        ] {
            let _ = parse_addr(s);
            let _ = parse_hex16(s);
            let _ = whose(&[], s);
        }
        // A key of the wrong length must never be silently padded into a
        // working key — that would resolve nothing and look like bad hardware.
        assert!(parse_hex16(&"0".repeat(31)).is_none());
        assert!(parse_hex16(&"0".repeat(33)).is_none());
    }

    #[test]
    fn parses_what_bluez_writes() {
        // BlueZ writes the key as unbroken uppercase hex.
        assert!(parse_hex16("D11F4E076BB0085311AC04B5A0AB62B6").is_some());
        assert!(parse_hex16("too short").is_none());
        assert_eq!(parse_addr("EC:A9:07:A0:3A:60").unwrap()[0], 0xEC);
        assert!(parse_addr("nonsense").is_none());
    }
}
