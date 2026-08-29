# airspace ear — ESP32-C3

A five-euro board that listens to Bluetooth and reports what it hears to a
collector over Wi-Fi.

The point is not that it hears better than a laptop — it hears worse. The point
is that it does not move. One receiver hears a distance and never a direction;
three receivers at known positions hear a place. A laptop is a poor third ear
because it goes where you go. A board taped to a shelf is an excellent one.

## What it does

Passive BLE scan, continuously. Every two seconds it collapses what it heard to
one observation per address and POSTs a batch to the collector's `/ingest`,
authenticated with the shared token.

It never advertises, never connects, and never sends a scan request — the scan
is passive, so the board is an ear rather than something that can be located by
what it transmits. (The advertising code is compiled in because NimBLE will not
build without it; see the note in `sdkconfig.defaults`. It is never called.)

## The interesting constraint

The C3 is single-core and this firmware wants both radios at once: a continuous
BLE scan and a live Wi-Fi association. That is what the coexistence layer is
for, and it is the entire reason this is ESP-IDF rather than something more
pleasant to write. Rust on the C3 is genuinely nice — it is RISC-V, so the
stable toolchain works with no Xtensa fork — but `esp-wifi`'s coexistence is the
least-trodden part of that ecosystem, and coexistence is precisely the risk
here.

Wi-Fi power save is disabled deliberately. A modem that sleeps mid-scan is the
classic way a coexisting BLE scan quietly starts missing most of the room.

## Build and flash

```
. ~/esp/esp-idf/export.sh
idf.py menuconfig       # → "airspace ear": SSID, password, collector URL, token, position
idf.py build
idf.py -p /dev/ttyACM0 flash monitor
```

`sdkconfig` is gitignored, because after `menuconfig` it contains your Wi-Fi
password and the ingest token.

## Things that will bite

**The collector must be reachable.** It binds to loopback by default, which the
board cannot reach. Binding it to the LAN also exposes its web UI, which is not
authenticated — decide that deliberately rather than by accident.

**The clock has to be real.** The collector uses each observation's timestamp to
decide what is stale, so a board sitting at 1970 reports devices that are
silently discarded. The firmware waits for SNTP before posting anything and says
so in the log rather than failing quietly.

**A 404 from the collector means the token is wrong.** The collector answers 404
rather than 401 to a bad token on purpose, so that a stranger probing the port
cannot tell a wrong secret from a wrong path. It does mean this one error is
less obvious than it should be, hence the log line.

**Addresses are byte-reversed.** The controller hands them over least-significant
octet first and BlueZ prints them the other way. Getting it backwards produces a
device that never matches the same device seen by another node — which looks
like a distance problem, not a formatting one.
