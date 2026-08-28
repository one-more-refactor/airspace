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
    /// dBm the device claims to transmit at. Present on most advertisers and
    /// worth having: assuming every radio shouts equally loudly is the single
    /// biggest error in an RSSI distance estimate. A tracker at 0 dBm and a
    /// beacon at +12 differ by a factor of three in apparent range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_power: Option<i16>,
    /// Bluetooth Class of Device — a packed major/minor category. Only classic
    /// and dual-mode devices have one, and BlueZ only fills it in once it has
    /// interrogated the device, so it is a bonus rather than a source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<u32>,
    /// BlueZ's own guess at an icon name — "phone", "audio-headphones".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// "bluetooth:v004Cp200EdD415" — vendor, product and device revision. This
    /// is the exact model, not a category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modalias: Option<String>,
    /// GAP advertising flags. Bit 2 clear means the device is LE-only, which
    /// separates a modern tag or wearable from a dual-mode phone or laptop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<u8>,
    /// Which radio heard this — "ble" or "wifi". Defaulted rather than required
    /// so captures written before the Wi-Fi ear existed still parse.
    #[serde(default = "ble")]
    pub src: String,
    /// What the frame or advertisement says the device is doing, when the
    /// protocol says so in the clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<String>,
}

fn ble() -> String {
    "ble".to_string()
}

/// Distance for Wi-Fi, which is a different radio problem from Bluetooth: far
/// more transmit power, so a client at one metre reads around -40 dBm rather
/// than -59. The exponent is higher too, because Wi-Fi is habitually used
/// through more walls than a pair of earbuds is.
pub fn wifi_metres(rssi: i16) -> f32 {
    const AT_ONE_METRE: f32 = -40.0;
    const EXPONENT: f32 = 3.0;
    10f32.powf((AT_ONE_METRE - rssi as f32) / (10.0 * EXPONENT))
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
    10f32.powf((TX_AT_ONE_METRE - rssi as f32) / (10.0 * PATH_LOSS_EXPONENT))
}

const PATH_LOSS_EXPONENT: f32 = 2.5;
/// Path loss at one metre, measured rather than derived.
///
/// Free-space theory says 40 dB at 2.4 GHz. Real BLE radios lose another ~19 dB
/// to antenna inefficiency, casework and the receiver front end, and the
/// industry reference point everybody actually uses — 0 dBm read as -59 dBm at
/// one metre — implies 59. Using the theoretical figure put a device at 302 m
/// that the fixed model put at 30, purely because it advertised a high transmit
/// power; the calibrated estimate has to agree with the uncalibrated one at the
/// reference point or it is not a refinement, it is a second unit.
const LOSS_AT_ONE_METRE: f32 = 59.0;

/// Distance using the power the device says it transmits at, rather than the
/// power every device is assumed to transmit at.
///
/// Better, not good. The exponent still dominates and a body in the way still
/// costs a factor of two and a half. But it removes a real error: the fixed
/// model silently treats a +12 dBm beacon and a 0 dBm tag at the same RSSI as
/// being the same distance away, and they are not.
///
/// Implausible values are ignored rather than trusted — some devices put
/// nonsense in the field, and a confident wrong number is worse than the
/// honest assumption.
pub fn metres_with_tx(rssi: i16, tx_power: Option<i16>) -> (f32, bool) {
    match tx_power {
        Some(p) if (-40..=20).contains(&p) => {
            let loss = p as f32 - rssi as f32;
            (
                10f32.powf((loss - LOSS_AT_ONE_METRE) / (10.0 * PATH_LOSS_EXPONENT)),
                true,
            )
        }
        _ => (rough_metres(rssi), false),
    }
}

/// Class of Device: bits 8-12 are the major class, 2-7 the minor. The major
/// class alone answers "is that a phone or a fridge".
pub fn device_class(class: u32) -> Option<&'static str> {
    let major = (class >> 8) & 0x1F;
    let minor = (class >> 2) & 0x3F;
    Some(match (major, minor) {
        (1, 3) => "laptop",
        (1, _) => "computer",
        (2, 3) => "smartphone",
        (2, 4) => "phone, wired-modem class",
        (2, _) => "phone",
        (3, _) => "network access point",
        (4, 1) => "wearable headset",
        (4, 2) => "hands-free device",
        (4, 6) => "headphones",
        (4, 7) => "portable speaker",
        (4, 8) => "car audio",
        (4, _) => "audio or video device",
        (5, 0x10) => "keyboard",
        (5, 0x20) => "mouse",
        (5, 0x30) => "keyboard and mouse",
        (5, _) => "peripheral",
        (6, _) => "imaging device — display, camera, scanner or printer",
        (7, _) => "wearable",
        (8, _) => "toy",
        (9, _) => "health device",
        _ => return None,
    })
}

/// GAP advertising flags, of which one bit is worth reading: whether the
/// device supports classic Bluetooth at all.
pub fn radio_kind(flags: u8) -> &'static str {
    // Bit 2 (0x04) set means BR/EDR is NOT supported.
    if flags & 0x04 != 0 {
        "low-energy only — a tag, a wearable or a sensor rather than a phone"
    } else {
        "dual-mode — supports classic Bluetooth, so a phone, laptop or headset"
    }
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
    fn the_two_distance_models_agree_at_the_reference_point() {
        // A 0 dBm device read at -59 dBm is one metre away under both models.
        // If this drifts, one of them is silently using different units.
        let (with_tx, calibrated) = metres_with_tx(-59, Some(0));
        assert!(calibrated);
        assert!((with_tx - rough_metres(-59)).abs() < 0.01);
        assert!(with_tx > 0.95 && with_tx < 1.05);
    }

    #[test]
    fn a_louder_transmitter_heard_equally_faintly_is_further_away() {
        let (quiet, _) = metres_with_tx(-80, Some(0));
        let (loud, _) = metres_with_tx(-80, Some(12));
        assert!(loud > quiet, "a +12 dBm device at -80 must be further than a 0 dBm one");
    }

    #[test]
    fn nonsense_transmit_power_falls_back() {
        // Some devices put junk in the field. A confident wrong number is worse
        // than the honest assumption.
        let (m, calibrated) = metres_with_tx(-70, Some(127));
        assert!(!calibrated);
        assert!((m - rough_metres(-70)).abs() < 0.01);
    }

    #[test]
    fn reads_the_device_class() {
        // 0x00240418 — what the AirPods on this desk actually report.
        assert_eq!(device_class(0x00240418), Some("headphones"));
        assert_eq!(device_class(0x0), None);
    }

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
