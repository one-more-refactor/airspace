# earshot

What the devices around you say out loud, without being asked.

> **Alpha — v0.1.0-alpha.** Bluetooth only so far. Interfaces and output format
> will change.
>
> **Provenance:** written by Claude Code, from my design brief and against my
> hardware. Tested, but not audited line by line by a human. It writes other
> people's identifiers to a file on your disk — read it before you run it.

```
$ earshot watch 600
$ earshot report
8 addresses, ~7 devices, 214 sightings → earshot.html
```

The page it produces is the point: distance rings, a timeline of when each
address was audible, and a card per device listing what it announced about
itself. One self-contained HTML file, openable from a USB stick with no
internet, because it exists to be shown to somebody who does not believe you.

## The argument

There is no attack here. No monitor mode, no injection, no root, nothing
transmitted. An unprivileged user account reads the Bluetooth advertising
channels that every desktop session can already read, and writes down what
arrives.

That is the whole point. This is the **floor** — the ambient, default,
no-special-hardware case — and the floor already includes:

* a count of the devices near you, updated every two seconds;
* a distance band for each one, good enough to separate *this room* from
  *through a wall*;
* the manufacturer of most of them;
* on Apple hardware, an unencrypted message type that says what the device is
  currently doing: a share sheet is open, Siri is listening, a watch is nearby,
  earbuds are in someone's ears, the phone's screen just unlocked;
* for every device that has not implemented address randomization, a permanent
  identifier for a specific physical object.

Published de-randomization research does considerably better than this using
frame timing, sequence numbers and payload field ordering — none of which bluez
even exposes. Whatever earshot finds, a determined observer finds more.

## What it deliberately does not claim

**Bearing.** One antenna cannot hear a direction. The page places devices at an
arbitrary but stable angle and says so on the page. Anything plotting a compass
bearing from a single receiver is showing you a decoration.

**Distance.** Signal strength through a log-distance path-loss model. A human
body between you and a device costs about 10 dB, which at these constants is a
factor of 2.5 in apparent distance. The rings mean "arm's reach / same room /
through a wall / somewhere in the building" and nothing finer.

**Identity across rotation.** Addresses are linked only when the advertising
fingerprint matches *and* the lifetimes do not overlap *and* the new address
appears within 30 seconds of the old one going quiet. It is the crudest rule
that works, chosen so that every link on the page can be checked by hand.

## Wi-Fi

Not implemented, and the reason is worth knowing before you go looking for it.

Wi-Fi is the richer source. A phone that is not currently connected to anything
sends probe requests asking after networks it has joined before — which is a
list of the places its owner has been. Reading those needs the radio in monitor
mode, and most laptop Wi-Fi chips will not do it. Check yours:

```
$ earshot doctor
```

If it says no, a USB adapter on `mt7921au` or `mt7612u` is the usual fix. The
capture side would be a second listener writing the same observation format;
the analysis and the page would not need to change.

## Install

```
cargo build --release
install -Dm755 target/release/earshot ~/.local/bin/earshot
```

Needs a working BlueZ. No root, no capabilities, no group membership.

## A word about the capture file

`~/.local/share/earshot/observations.jsonl` is a log of who was physically near
you, with timestamps. Most of it is your neighbours. It is exactly the file this
tool exists to warn people about, so:

* it is gitignored, and it should stay that way;
* do not publish a report generated from it without redacting the addresses;
* delete it when you are done making your point.

## License

GPL-3.0-or-later.
