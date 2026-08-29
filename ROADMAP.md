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
* **It requires identity resolution on the node.** You cannot allowlist a
  rotating address by address. Each node needs the IRK and has to run the
  resolution itself — the C3 has hardware AES, so this is cheap, but it means
  key material on a board on a shelf. That is a real trade and the reason to
  keep the keys to devices you own.

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

### The config flow is the interesting part

This is where a presence system is won or lost, because every hard problem in
this project is a placement or calibration problem wearing a costume.

**Real calibration, not assumed constants.** Distance is currently a path-loss
model with textbook numbers: −59 dBm at one metre, exponent 2.5. Both are
guesses, and they are per-room, per-device and per-node wrong. The flow should
measure instead: hold the device at one metre from a node, then three, then
some distance that matters to you, and solve for the reference power and the
exponent that fit *your* walls and *that* device. Store it per (node, device).
This is the single change that would move distance from "a band" to "a number".

**Measure the link, not just the radio.** Advertisements arrive at a rate; a
badly placed node hears a fraction of them. Report packets per second and drop
rate per node per device during setup, because "this node is in a bad spot" and
"this node is fine" look identical in a static reading and obvious in a rate.

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
