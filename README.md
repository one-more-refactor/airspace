# airspace

What the devices around you say out loud, without being asked — as a live map.

> **Alpha — v0.1.0-alpha.** Bluetooth only so far. Interfaces and output format
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

Needs a working BlueZ. No root, no capabilities, no group membership.

## A word about the capture file

`~/.local/share/airspace/observations.jsonl` is a log of who was physically near
you, with timestamps. Most of it is your neighbours. It is exactly the file this
tool exists to warn people about, so:

* it is gitignored, and it should stay that way;
* do not publish a report generated from it without redacting the addresses;
* delete it when you are done making your point.

## License

GPL-3.0-or-later.
