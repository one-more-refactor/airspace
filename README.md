# airspace

What the devices around you say out loud, without being asked — as a live map.

> **Alpha — v0.1.0-alpha.** Bluetooth and Wi-Fi. Interfaces and output format
> will change.
>
> **Provenance:** written by Claude Code, from my design brief and against my
> hardware. Tested, but not audited line by line by a human. It writes other
> people's identifiers to a file on your disk — read it before you run it.

```
$ airspace serve
airspace: http://127.0.0.1:9970
```

Open it and you get the room: every device audible right now, how far away it
probably is, and what it announced about itself while announcing its presence.

## The console

```
airspace-console
```

One screen: the room, with the things in it. Modes are overlays rather than
tabs, so you never lose your bearings.

```
┌──────────────────────────────────────────┐  philip's phone
│              ············                │    in this room
│         ·····            ·····           │    3.2 m · estimated
│      ···                      ···        │    ▰▰▰▰▰ carl
│     ··          ◈ carl           ··      │
│      ···                      ···        │  airpods
│         ·····            ·····           │    next room, or through a wall
│  ◆ ear1      ············                │    ▰▱▱▱▱ carl
└──────────────────────────────────────────┘      only catches it occasionally
 tab next node · m move it · c measure · q quit
```

**Devices are rings, not dots.** With one node that is genuinely all that is
known, and a dot would be an invention. The rings tighten into a point as ears
are added, so the picture gets more certain in the same way the system does.

**Placing a node is moving it.** Press `m` and the arrow keys move the marker
while the floor shades green where the geometry is good and red where it is
not. You do not need to know what dilution of precision means to find a decent
spot — you move the thing until the room goes green. `s` jumps it to the best
place the geometry can find, which is a suggestion and says so, because it knows
nothing about where there is a plug socket.

**The first line is a sentence.** "at your desk, estimated" rather than
"3.2 m basis=advertised". The numbers are underneath for when you want them,
and the *basis* is always shown, because a measured distance and a guessed one
look identical if you only print the number.

**Measuring replaces the guessing.** `c` walks you to one, two, four and eight
metres, takes a reading at each, and fits your actual walls. The result is
described in words — "normal for a room with furniture in it", "something solid
is in the way" — not as a path-loss exponent you have to have an intuition for.

**The footer only offers keys that do something right now.** A key you have to
learn to ignore is a key that should not have been there.

Nothing it writes touches a file you maintain: placements go to
`placement.toml` and measurements to `calibration.toml`, both read by the
collector and written only by the console — which is also why the daemon can
keep `ProtectHome=read-only`.

## Tracking your own devices, not the building

By default airspace reports **only** devices you have configured an identity
for. A stranger's device is dropped at ingest, before it reaches state, the page
or the disk — so there is nothing to delete afterwards and no policy to trust.
Set `general.track_only_known = false` to see everything again, which is what
`airspace watch` and `airspace report` are for.

It also refuses to treat an earbud's distance as comparable while it is out of
an ear. In an ear, in a pocket, on a desk and in a case are four different radio
situations, and averaging across them produces a number that is wrong in all
four.

## One node is a supported configuration

A single machine with a Bluetooth adapter is enough for presence: is that device
here, how far away, is it moving. Locking the screen when you stand up needs no
network, no second board and no infrastructure. Extra ears buy *direction*, and
nothing else — so they are an upgrade, not a prerequisite.

The rest of the plan is in [ROADMAP.md](ROADMAP.md).

## Direction, honestly

**One antenna cannot hear a bearing.** It hears a signal strength, which is a
bad estimate of a distance and nothing else. Any single-receiver tool that draws
a compass arrow is drawing a decoration.

So airspace draws the thing that is actually known: the set of places a device
could be, given what every listening node heard. That set has a shape, and the
shape is the honesty —

| nodes | what you get | what it looks like |
|---|---|---|
| 1 | a distance | a ring around the node |
| 2 | two candidate places | two blobs where the rings cross |
| 3+ | a position | a point, with the residual cloud around it |

The picture degrades into vagueness instead of into a confident lie, and you can
see at a glance how much a reading is worth.

Getting to three is the whole design of the tool. Any machine with a Bluetooth
adapter can be an ear:

```
# on the collector
$ airspace serve 0.0.0.0:9970

# on each other machine, with its position in ~/.config/airspace/config.toml
$ airspace feed http://collector:9970/ingest
```

Ingest is token-authenticated and plain HTTP — meant for a LAN or a tailnet, not
the open internet. An unauthenticated ingest endpoint lets anyone on the network
draw imaginary people into your room, so it refuses to run without a token
rather than degrading quietly.

## The argument

There is no attack here. No monitor mode, no injection, no root, nothing
transmitted. An unprivileged user account reads the Bluetooth advertising
channels that every desktop session can already read, and writes down what
arrives.

That is the point. This is the **floor** — the ambient, default,
no-special-hardware case — and the floor already includes:

* a live count of the devices near you, updated every two seconds;
* a distance band for each, good enough to separate *this room* from *through a
  wall*;
* the manufacturer of most of them;
* on Apple hardware, an unencrypted byte saying what the device is currently
  doing: a share sheet is open, Siri is listening, a watch is nearby, earbuds
  are in someone's ears, the screen just unlocked, this is a Find My beacon;
* for every device that has not implemented address randomization, a permanent
  identifier for a specific physical object.

Published de-randomization research does considerably better than this using
frame timing, sequence numbers and payload field ordering — none of which BlueZ
even exposes. Whatever airspace finds, a determined observer finds more.

## Wi-Fi

Bluetooth tells you something is present. Wi-Fi tells you rather more — every
device associated to an access point announces itself constantly, including
laptops, televisions and consoles with no Bluetooth presence at all.

```
sudo setcap cap_net_raw+ep ~/.local/bin/airspace
sudo iw phy phy0 interface add mon0 type monitor
sudo ip link set mon0 up && sudo iw dev mon0 set channel 6
airspace wifi mon0
```

It feeds the same collector as everything else, so Wi-Fi devices appear on the
same map with the same distance rings.

**This is the one part that needs a privilege.** A raw packet socket requires
`CAP_NET_RAW`. The Bluetooth half deliberately needs nothing at all, and folding
the two into one process would make the whole tool demand a capability for the
sake of one of its two radios — so the Wi-Fi ear is a separate command you grant
separately, or do not run.

### What a frame actually gives up

Measured on an RTL8852BE, which several wikis say cannot do monitor mode and
which does it fine under the in-kernel `rtw89`:

* the transmitter's MAC, and whether it is randomised — the locally-administered
  bit is the same public/private split Bluetooth has, one bit in both;
* signal strength, per antenna;
* what the device is doing: probing, beaconing, or carrying traffic;
* network names from beacons and probe responses.

### The probe-request myth, corrected

The widely repeated claim is that a phone broadcasts every network it has ever
joined, making a probe request a travel history. That was true and is now mostly
not: modern iOS and Android send **wildcard** probes with no name in them. The
first capture on the machine this was built against contained exactly that — a
probe request with an empty SSID.

The leak is still real for older devices, laptops, IoT things and anything
hunting a hidden network. But it is a minority of devices now, not all of them,
and a tool that assumes otherwise is selling a scare rather than a measurement.
airspace records the names it actually sees and claims nothing about the rest.

### Channels

A monitor interface hears one channel at a time, so every device is reported
with the channel it was heard on — that number is as much a statement about what
the listener could hear as about the device. Hopping needs `CAP_NET_ADMIN` on
top, and airspace deliberately does not reconfigure your radio behind your back.

## Configuration

`~/.config/airspace/config.toml` — see `airspace.toml.example`.

```toml
[room]
width = 10.0      # metres; only used to draw the picture
height = 8.0

[node]
name = "carl"     # this ear
x = 5.0           # metres from the top-left corner
y = 4.0

[collector]
url = ""          # where `airspace feed` sends observations
token = ""        # shared secret; ingest refuses to run without one
```

## Commands

```
airspace serve [BIND]      listen and serve the live picture (default 127.0.0.1:9970)
airspace feed [URL]        listen here, report to a collector
airspace wifi IFACE        listen on a monitor interface too (needs CAP_NET_RAW)
airspace watch [SECONDS]   append to a capture file instead
airspace report [OUT.html] render a capture as a standalone page
airspace doctor            what this machine's radios can and cannot hear
```

`report` produces one self-contained HTML file with no scripts and no network
calls, openable from a USB stick — because it exists to be shown to somebody who
does not believe you.

## Install

```
cargo build --release
install -Dm755 target/release/airspace ~/.local/bin/airspace
```

Needs a working BlueZ. The Bluetooth half needs no root, no capabilities and no
group membership. The Wi-Fi half needs `CAP_NET_RAW` and a monitor interface,
and is a separate command precisely so that stays true of everything else.

## A word about the capture file

`~/.local/share/airspace/observations.jsonl` is a log of who was physically near
you, with timestamps. Most of it is your neighbours. It is exactly the file this
tool exists to warn people about, so:

* it is gitignored, and it should stay that way;
* do not publish a report generated from it without redacting the addresses;
* delete it when you are done making your point.

## License

GPL-3.0-or-later.
