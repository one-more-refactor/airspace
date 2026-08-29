# Roadmap

Where this is going, and why. Nothing here is built yet. It is written down so
the decisions are arguable rather than implicit.

The short version of the shift: **airspace stops being a tool that watches the
building and becomes a tool that tracks your own devices.** Those are different
products with different ethics, and the second one is smaller, faster and
better.

---

## 1. Track only what you asked it to track

Today every advertiser in range is recorded — currently sixteen to twenty-one
devices, almost all of them neighbours. For presence tracking that is not
signal, it is cost: RAM on a board with 400 kB of it, bandwidth on a link that
should be carrying under a hundred bytes a second, and a page you have to read
past to find your own phone.

**The change:** an allowlist. Nodes report only devices matching a configured
identity, and drop everything else before it leaves the radio.

Three consequences worth stating plainly:

* **Bandwidth collapses.** Two or three devices at seven bytes each is ~20 bytes
  per sweep. That is what makes a BLE-advertising transport comfortable rather
  than marginal.
* **The privacy posture inverts.** Right now the capture file is a log of who
  was near you, and the README has to tell people to delete it. With an
  allowlist there is nothing to delete: unrecognised devices are never written
  down. The tool stops being something that needs an apology.
* **It does NOT have to put keys on the nodes.** An earlier draft of this said
  each node would need the IRK to allowlist a rotating address. That was the
  wrong conclusion — see "Where the keys live" below. Resolution stays on the
  collector; nodes send what they hear and the filtering happens where the
  secret already is.

The "look what leaks by accident" demonstration does not disappear — it becomes
`airspace watch` and `airspace report`, an explicit mode you turn on to make a
point, rather than what the tool does by default.

## 2. A TUI, and the web page demoted

The web UI is a visualisation for one person on one machine. The product is a
terminal interface.

**Go with Bubble Tea.** Not because Go beats Rust here, but because the boundary
is already clean: the collector speaks HTTP and JSON, so the TUI is a client and
the language choice costs nothing architecturally. Bubble Tea's model for
multi-step wizards is genuinely better than ratatui's, and the config flow below
is mostly a wizard. The cost is a second toolchain in the repo, which is worth
naming and is the only real argument for staying in Rust.

### It is an operations console, not a setup wizard

A wizard runs once and lies forever afterwards. Calibration drifts: furniture
moves, a door closes, a node gets nudged, a battery sags and the transmit power
with it. The interesting version of this is a thing you leave open, that keeps
measuring and tells you when the picture stopped being true.

So: initial calibration and continuous calibration are the same code path, and
the difference is only whether you are being prompted. Everything below runs
live, all the time, and the setup flow is just the first five minutes of it.

**Real calibration, not assumed constants.** Distance is currently a path-loss
model with textbook numbers: −59 dBm at one metre, exponent 2.5. Both are
guesses, and they are per-room, per-device and per-node wrong. The flow should
measure instead: hold the device at one metre from a node, then three, then
some distance that matters to you, and solve for the reference power and the
exponent that fit *your* walls and *that* device. Store it per (node, device).
This is the single change that would move distance from "a band" to "a number".

**Measure the link continuously, not just at setup.** Advertisements arrive at
a rate, and a badly placed node hears a fraction of them. Packets per second,
inter-arrival gaps and drop rate per node per device — because "this node is in
a bad spot", "this node has died" and "this node is fine" look identical in a
static reading and are obvious in a rate.

This is also the only honest health check the system can have. Everything that
has gone wrong in this project so far — the beacon blind after suspend, the
collector alive with a dead D-Bus connection, the firmware logging to pins with
no cable on them — presented as *silence*, and silence is indistinguishable
from a quiet room. A live rate display makes the failure visible without
anybody having to suspect it first.

**Debug the links, not just report them.** Which node last heard each device
and how long ago; whether a node is reporting but hearing nothing; whether the
collector is ingesting but the radio has stopped. Named states rather than an
absence of rows.

**Help place the nodes.** Two ears in a line give you almost nothing; two ears
spread across the room give you intersecting rings. This has a real name —
geometric dilution of precision, borrowed from GPS — and it can be computed for
a candidate layout and shown as a score before anyone tapes anything to a shelf.
Guiding placement is worth more than any amount of filtering afterwards.

## 3. Trust in-ear state, and only measure when it is true

An in-ear bud is not at a fixed position relative to its owner. In an ear, in a
pocket, on a desk and in a case are four very different radio situations, and
averaging across them produces a distance that is wrong in all four.

Apple broadcasts in-ear state unencrypted in the proximity advertisement. Use
it: measure when worn, ignore the reading otherwise, and say on screen which is
happening. `vanish`'s beacon already gates on this for exactly this reason, and
the same logic belongs here.

## 4. Home Assistant, later

One process holds the MQTT connection and publishes, with discovery messages so
entities appear on their own. Default to the host PC because it is already
awake and already has the collector; fall back to electing a node when the host
is not available.

Deliberately last. It is an export, and exports are easy once the thing being
exported is correct.

## Where the keys live

Prompted by a good question: how does ESPHome handle this?

**It does not, in the sense you would hope.** `secrets.yaml` is compile-time
substitution — ESPHome replaces `!secret wifi_password` with the literal value
when it builds, and the device only ever sees the substituted firmware. So
`secrets.yaml` protects your git repository, not your device. The API
encryption key is a 32-byte pre-shared key used with the Noise protocol to
encrypt the device-to-Home-Assistant channel; that is transport security, and
it says nothing about data at rest.

ESPHome's own security guidance is candid about the consequence. It states that
physical access to a device allows an attacker to "extract encryption keys and
passwords from flash memory", and it offers no flash encryption or secure boot
guidance at all. The threat model is explicitly a trusted network, and the
mitigation for physical access is device placement. In short: **ESPHome's
answer to a key on a shelf is "do not let anyone take the shelf".**

ESP-IDF does offer more, and ESPHome simply does not turn it on: flash
encryption (AES-XTS under a key burned into eFuse, so a serial dump yields
ciphertext) and secure boot v2. Both are irreversible once burned and make
development genuinely painful, which is why nobody defaults to them.

### What this means here

There are three options for an IRK on a node, and the third is best:

1. **Accept it**, as ESPHome does. Physical security by placement.
2. **Burn flash encryption.** Real protection, irreversible, every reflash gets
   harder. Justified for a key that identifies a person, arguably.
3. **Never put the key on the node.** The node reports what it hears; the
   collector resolves. The secret stays on the machine that already has a
   locked screen, a disk password and a threat model.

Option 3 is what airspace does today, and the only reason the roadmap drifted
away from it was the bandwidth argument for filtering at the node — which does
not survive contact with the numbers. Unfiltered is about 112 bytes per sweep,
and BLE 5 extended advertising carries roughly 250 bytes per PDU. It fits. The
saving was never worth putting a key that identifies a person onto a board that
lives on a shelf.

If node-side filtering ever does become necessary, the version that does not
leak the key is for the collector to push *the currently resolved address* to
the nodes and let them filter on that until it rotates. That has a real
bootstrapping hole — a device only a distant node can hear never gets resolved
in the first place — which is exactly the kind of thing to find out by
measuring rather than by arguing.

---

## Considered and rejected

### Extending an audio link through an ESP

The idea: keep the AirPods connected to the PC, and have a board pick them up
when you walk out of the PC's range.

**Not possible, and not for a software reason.** Two separate blockers:

*The radio is absent.* `esptool` reported this board as `Features: Wi-Fi, BT 5
(LE)`. The ESP32-C3 has no BR/EDR at all — no A2DP, no HFP, no Classic. An
audio link is not something it can carry under any firmware. The original ESP32
has Classic, but that only moves you to the second blocker.

*Bluetooth has no handover.* There is no roaming, no relay, no repeater in the
specification. A board could act as an A2DP sink and re-transmit as a source,
but then the buds are paired to the board rather than the PC, the audio takes a
decode/re-encode hop, and nothing hands the stream over as you walk — you would
be manually switching devices, which is what you already can do.

**But the useful half of that request already works.** If what you want is for
presence tracking to continue when you leave the PC's range, that is the entire
design: a node hears the AirPods and reports RSSI whether or not the PC can hear
them. The gap is not range, it is *identification* — right now eleven Apple
devices are advertising here and nine are on rotating addresses, so a node
cannot tell your buds from a neighbour's without a key.

Which makes the AirPods IRK the concrete unblocker:

```
sudo grep -A2 IdentityResolvingKey \
  /var/lib/bluetooth/50:2E:91:74:A4:A1/1C:B3:C9:C5:44:14/info
```

### Bluetooth Mesh as the transport

Skipped, agreed. Mesh's value is multi-hop relay, and in a flat every node is
within direct range of the collector. Plain BLE advertising carries the traffic
with none of the provisioning, and the collector is already scanning
continuously — it is a receiver that exists.

Revisit only if a node turns out to be out of BLE range, which is the one
situation where routing earns its complexity.
