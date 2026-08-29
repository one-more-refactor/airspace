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

Full screen, chunky, and it moves. The design rule is that **you should never
need to be told what a word means to use it** — and the second rule, learned
from the measuring screen, is that a tool you walk around a flat with has to be
readable from across that flat.

```
        ███ █ █ ███ █   ███ ███  █  ███     ███ █ █ ███ ██  ███
         ███ ███  █  █    █  ███     ███     ███ ███ █ █ █ █ ██
        █   █ █ ███ ███ ███ █       ███     █   █ █ ███ █ █ ███

                IN THIS ROOM   ·   3.2 m   ·   estimated
        ╺━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╸
              ┌─────────────────────────────────────┐
              │              ···········            │
              │          ···              ···       │
              │        ·         ◆ CARL      ·      │
              │          ···              ···       │
              │  ◆ EAR1      ···········            │
              └─────────────────────────────────────┘

           CARL  ━━━━━━━━━━━━━━━━━━━━  hears it consistently
           EAR1  ━━━━━━━━━━━━──────────  most of the time

              ▸ PHILIP'S PHONE     AIRPODS     WATCH
```

### Why measuring asks you to keep moving

RSSI calibration is rough, and the reason is physics rather than software.
Standing still samples **one** multipath realisation: at 2.4 GHz the wavelength
is 12 cm, so constructive and destructive interference swing the reading by
±10 dB across the width of your hand. Averaging for longer in the same spot
removes receiver noise and leaves that error entirely untouched.

Walking does remove it. A slow arc at the target radius sweeps through many
realisations, and their median is the actual path loss at that distance. So the
whole ten seconds is the sampling window, the instruction is to circle rather
than stand, and the **spread** is reported at the end — a suspiciously tight
spread usually means you stood still, and the number is worth less than it
looks.

Stations adapt to the room. Eight metres in a small flat is a reading taken
through a wall, which is not the same propagation as the other three and drags
the exponent up, quietly widening every distance the model later reports. A
10 × 8 m room gets 1 / 1.9 / 3.5 / 6.6 m; a 4 × 3 m one gets 1 / 1.4 / 2.1 / 3.

The honest ceiling: this gets you to roughly a metre or two, which is what
room-level presence needs. Sub-metre positioning is not available from signal
strength at any amount of care — that needs angle-of-arrival (Bluetooth 5.1
direction finding) or ultra-wideband, both of which are different hardware.

**Measuring is a run you walk, not a key you press.** You cannot press enter
while standing eight metres away holding your phone, so you press it once and
the screen takes over: ripples expand from the node, a countdown you can read
from across the room ticks down, and at zero it captures and walks you to the
next station by itself. Four stations, about forty-five seconds, hands-free
after the first press. The live signal bar moves as you walk, so you can see
the thing being measured respond to you.

**Everything animates.** Rings breathe, the selected node pulses, the headline
brightens when a device comes close. A static picture of a live system is one
that can be stale without looking stale — and going silently stale is the
failure this project has met more than any other.

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

**Measuring replaces the guessing.** `c` takes the whole screen and turns
calibration into a run: press enter *once*, then put the keyboard down. It
counts you down to one, two, four and eight metres in turn, takes the reading
when the number hits zero, and fits your actual walls.

One keypress is the whole design. Calibrating means standing a known distance
away holding the device, which is exactly the moment you cannot reach the
keyboard — so the old version, which asked you to press enter at each station,
was asking you to be in two places at once. The signal bar moves as you walk,
so you can see the thing being measured respond to you, and the reading is the
median of the last few seconds rather than whichever advertisement landed on
the buzzer. The result is described in words — "normal for a room with
furniture in it", "something solid is in the way" — not as a path-loss exponent
you have to have an intuition for.

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
