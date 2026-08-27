//! What an observation is, and what can be read off one without asking anybody.

use serde::{Deserialize, Serialize};

/// One sighting of one address at one moment. Deliberately flat and boring:
/// this is the raw material, and every claim the report makes has to be
/// derivable from a pile of these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// Unix seconds.
    pub t: u64,
    pub addr: String,
    /// "public" (burned into the radio, permanent) or "random" (rotating).
    pub at: String,
    pub rssi: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Bluetooth SIG company IDs present in the manufacturer data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub company: Vec<u16>,
    /// (company, first byte of that company's payload). For Apple and Microsoft
    /// the first byte is a message type, and the type alone says what the
    /// device is doing. The company travels WITH the byte on purpose: 0x01
    /// means "Swift Pair" from Microsoft and something else entirely from
    /// Apple, and a flat list of bytes silently invites that mix-up.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cmsg: Vec<(u16, u8)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paired: bool,
}

/// Bluetooth SIG company identifiers, the ones you actually meet in a flat.
/// Not the full list — the full list is 3000 entries of industrial sensors and
/// would bury the interesting answer, which is "whose phone is that".
pub fn vendor(id: u16) -> Option<&'static str> {
    Some(match id {
        0x004C => "Apple",
        0x0006 => "Microsoft",
        0x00E0 => "Google",
        0x0075 => "Samsung",
        0x0087 => "Garmin",
        0x000F => "Broadcom",
        0x0059 => "Nordic Semiconductor",
        0x000D => "Texas Instruments",
        0x0157 => "Huawei",
        0x038F => "Xiaomi",
        0x02E5 => "Espressif",
        0x0110 => "Sonos",
        0x009E => "Bose",
        0x012D => "Sony",
        0x00C4 => "LG",
        0x0171 => "Amazon",
        0x02FF => "Tile",
        0x0499 => "Ruuvi",
        0x0001 => "Nokia (Bluetooth SIG #1)",
        0x0030 => "ST Microelectronics",
        0x0118 => "Logitech",
        0x03DA => "Fitbit",
        _ => return None,
    })
}

/// Apple's manufacturer-data message types. The first byte is the type, and it
/// is not encrypted, not authenticated, and not optional — every Apple device
/// in the room announces what it is busy with.
pub fn apple_message(t: u8) -> Option<&'static str> {
    Some(match t {
        0x05 => "AirDrop — a share sheet is open",
        0x07 => "proximity pairing — earbuds, incl. battery and in-ear state",
        0x08 => "\"Hey Siri\" is listening",
        0x09 => "AirPlay target",
        0x0A => "AirPlay source",
        0x0B => "Watch is nearby and paired",
        0x0C => "Handoff — an app is being handed between devices",
        0x0D => "instant hotspot request",
        0x0E => "instant hotspot answer, with battery and signal bars",
        0x0F => "nearby action — setup, transfer, Wi-Fi password sharing",
        0x10 => "nearby info — lock state, screen state, activity level",
        0x02 => "iBeacon — a fixed location beacon",
        0x12 => "Find My — offline finding beacon, part of Apple's crowd-sourced network",
        _ => return None,
    })
}

/// Microsoft's, of which there is essentially one worth knowing.
pub fn microsoft_message(t: u8) -> Option<&'static str> {
    Some(match t {
        0x01 => "Swift Pair — a device advertising itself for pairing",
        0x03 => "CDP beacon — Windows announcing a nearby-sharing identity",
        _ => return None,
    })
}

/// 16-bit service UUIDs that carry a payload people would not expect to be
/// public. The full assigned-numbers list is long and mostly sensors.
pub fn service_meaning(uuid: &str) -> Option<&'static str> {
    let short = uuid.get(4..8)?.to_ascii_lowercase();
    Some(match short.as_str() {
        "fd6f" => "Exposure Notification — a contact-tracing rolling identifier",
        "fe2c" => "Google Fast Pair — model ID, resolvable to a product name",
        "feed" => "Tile tracker",
        "fd44" => "Apple continuity service",
        "fe9f" => "Google/Nest",
        "fdf0" => "Google Nearby",
        "fe07" => "Microsoft Swift Pair",
        "180f" => "battery level",
        "180a" => "device information — make, model, firmware",
        "1812" => "human interface device — a keyboard or a mouse",
        _ => return None,
    })
}

/// A very rough metres-from-here, from the log-distance path loss model with
/// the constants everybody uses for BLE indoors.
///
/// It is worth being blunt about the error: a body between you and the device
/// costs about 10 dB, which at these exponents is a factor of two and a half in
/// distance. This number sorts devices into "in the room", "through a wall" and
/// "somewhere in the building". It does not do better than that, and any
/// interface that prints it to one decimal place is lying.
pub fn rough_metres(rssi: i16) -> f32 {
    const TX_AT_ONE_METRE: f32 = -59.0;
    const PATH_LOSS_EXPONENT: f32 = 2.5;
    10f32.powf((TX_AT_ONE_METRE - rssi as f32) / (10.0 * PATH_LOSS_EXPONENT))
}

pub fn band(rssi: i16) -> &'static str {
    match rssi {
        r if r >= -55 => "arm's reach",
        r if r >= -70 => "same room",
        r if r >= -85 => "next room, or through a wall",
        _ => "somewhere in the building",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_is_monotonic_and_sane() {
        assert!(rough_metres(-59) > 0.9 && rough_metres(-59) < 1.1);
        assert!(rough_metres(-90) > rough_metres(-70));
        assert!(rough_metres(-40) < 1.0);
    }

    #[test]
    fn knows_the_vendors_that_matter() {
        assert_eq!(vendor(0x004C), Some("Apple"));
        assert_eq!(vendor(0xFFFF), None);
    }

    #[test]
    fn reads_apple_message_types() {
        assert!(apple_message(0x07).unwrap().contains("in-ear"));
        assert!(apple_message(0x10).unwrap().contains("lock state"));
        assert_eq!(apple_message(0xEE), None);
    }

    #[test]
    fn service_uuids_are_matched_on_the_short_form() {
        assert!(service_meaning("0000fd6f-0000-1000-8000-00805f9b34fb").unwrap().contains("Exposure"));
        assert!(service_meaning("0000FE2C-0000-1000-8000-00805f9b34fb").unwrap().contains("Fast Pair"));
        assert_eq!(service_meaning("0000abcd-0000-1000-8000-00805f9b34fb"), None);
    }
}
