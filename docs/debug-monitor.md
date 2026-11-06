# The Debug-Monitor (UDP 7777)

The Miniserver can send its live debug output (what Loxone Config's *Debug Information* and the Loxone Monitor
show) as UDP packets. With `--monitor` (`VISLOX_MONITOR=1`), vislox receives them and shows them live in the Log
panel, marked *monitor*, above the system log that `lox log` reads. It is off by default.

## Turning it on

```bash
vislox ~/loxone-config --monitor                               # UDP 7777, this host's address on the route to the Miniserver
vislox ~/loxone-config --monitor --monitor-ip 192.168.1.82     # name the address yourself (Docker, several interfaces)
```

| Flag | Environment | |
|---|---|---|
| `--monitor` | `VISLOX_MONITOR=1` | Listen for the Debug-Monitor. |
| `--monitor-port` | `VISLOX_MONITOR_PORT` | The UDP port. It must be the Miniserver's Debug-Monitor port (`LoxWATCHPort`, 7777 unless changed in Loxone Config). |
| `--monitor-ip` | `VISLOX_MONITOR_IP` | The address the Miniserver sends to. Default: this host's address on the route to the Miniserver. In Docker, the Docker host's LAN address. |
| `--monitor-minutes` | `VISLOX_MONITOR_MINUTES` | How long a Start lasts (default 60). |
| `--monitor-strings` | `VISLOX_MONITOR_STRINGS` | A copy of the Miniserver's string table (`ENG.xml`), for lines that come as a string ID with parameters. |

vislox only keeps packets from the Miniserver's address (from lox's `host`). The Log panel counts the others and
names the address they came from. `--no-live` turns the Debug-Monitor off too.

## Starting and stopping the stream

The Miniserver sends nothing until it is told to. There are two ways.

**Start in the Log panel** (Miniservers on current firmware). vislox calls `dev/sps/log/<address>` with lox's user,
the same web service Loxone Config's *Debug Information* uses (and [Smarteon/lox-debug](https://github.com/Smarteon/lox-debug)).
The Miniserver then sends to that address, on its Debug-Monitor port. **Stop** calls `dev/sps/log`, which ends it.
vislox also stops it when it exits and when the time limit runs out. This changes nothing in the config, and the
program does not restart. It is a runtime switch, like the Debug Information toggle in Loxone Config. The Miniserver's
port setting is `LoxWATCHPort` in its `/sys/config.xml` (7777 on the house this was written against).

**A Monitor Server pointed at vislox** (older firmware). With the *Monitor Server* setting, the Miniserver asks that
server `GET http://<server>/?<serial>` (port 80) and follows the XML in the answer. vislox answers that request on its
own HTTP listener (`/?<serial>`, any other query gets the page):

```xml
<?xml version="1.0"?>
<Log Version="10" LogKeep="true" NoIPchange="false">
  <LogMode>on</LogMode>
  <LogDest>/dev/udp/192.168.1.82/7777</LogDest>
  <LogLevelCommon>info</LogLevelCommon>
  <LogLevelSPS>info</LogLevelSPS>
  <LogLevelProtocol>off</LogLevelProtocol>
  <LogLevelBus>info</LogLevelBus>
  <LogLevelFilesystem>off</LogLevelFilesystem>
  <LogLevelNet>off</LogLevelNet>
  <LogDay>1</LogDay>
</Log>
```

A request with another Miniserver's serial gets `<LogMode>off</LogMode>`. The levels are general and logic (SPS)
information plus Link/Tree bus messages (device restarts and reconnects), with no protocol, file system or network
detail. The stream ends after a day (`LogDay`, 7 at most). The Miniserver asks on port 80, so vislox must answer
there (`--listen 0.0.0.0:80`, or publish `80:7878` in Docker).

### Is there a Monitor Server setting on firmware 17?

Not that this house shows. On a Miniserver Gen 2 with firmware 17.2.10.21 (6 Nov 2026):

- The program config (`.Loxone`) has no Monitor Server attribute, on the Miniserver object or the document.
  Neither do any of the configs in lox's example library.
- The Miniserver's network settings are in `/sys/config.xml`, not the program config. It has `LogServer`
  (`log.loxone.com`, the crash-log server, with `EnableCrashlog`) and `LoxWATCHPort` (`7777`), but no Monitor Server.
  These are set in Loxone Config's Miniserver configuration dialog, not by uploading a program, so they cannot be
  a pending revision that Apply deploys.
- Loxone's *Required Ports & Domains* (March 2026) lists the Debug-Monitor as "UDP Port 7777 (Default), Port can be
  set in Config", and no longer mentions `monitorserver.loxone.com`.

So on current firmware, Start (`dev/sps/log/<ip>`) is the way. The Monitor Server answer is kept for Miniservers that
still have the setting.

## The packets

From sarnau's [Inside-The-Loxone-Miniserver](https://github.com/sarnau/Inside-The-Loxone-Miniserver)
(`Code/LoxoneMonitorServer.py`, firmware 10.x), implemented in `src/monframe.rs`:

- A frame starts with `0xFA1F` and a 16-bit size. Byte 4 is an XOR checksum over bytes 5…size. `0x1F1F` is padding.
  A datagram can hold several frames. Frames with a wrong checksum are dropped.
- A 28-byte header: parameter count, string ID, event counter, ms, Loxone time (seconds since 2009-01-01, the
  Miniserver's clock), source IP, used memory.
- String ID below `0xFFF0`: a log line. Its format string is in the string table (`ENG.xml`), followed by typed
  parameters: string, buffer, int, IP, MAC, TCP flags. vislox fills the format string in when it has one
  (`--monitor-strings`). Otherwise it joins the parameters, which on current firmware carry the text (one string
  parameter per line, as lox-debug relies on). A line repeated with the same event counter is shown once.
- `0xFFFF`: system stats (version 3). vislox shows CPU load, heap and Link errors in the Miniserver chip's details.
  `0xFFFE` (the Miniserver's own I/O) and `0xFFF0`–`0xFFFD` (the task list) are ignored.

The string table is not readable with lox's user on firmware 17. On 10.x it was `/sys/sys_ENG/ENG.xml`. On 17.x
there is `/sys/sys_ENG.zip`, and both HTTP (`dev/fsget`, 403) and FTP (550, permission denied) refuse it. That is
why `--monitor-strings` takes a local copy.

## The network

- The vislox host's firewall must accept inbound UDP on the port.
- It does not work over Remote Connect: the Miniserver sends to a LAN address.
- In Docker, publish the port as UDP (`docker/compose.yml` does: `7777:7777/udp`) and set `VISLOX_MONITOR_IP` to the
  Docker host's address. If Docker rewrites the sender's address, the Log panel says packets from another address were
  dropped: use `network_mode: host`.

## Turning it off

Click **Stop** in the Log panel (or let the time limit end it, or stop vislox), then start vislox without
`--monitor` (`VISLOX_MONITOR=0`). If a Monitor Server was set to vislox in Loxone Config, clear that setting there.

## Linking messages to blocks

Lines from both sources that carry a block's UUID (warnings do, e.g.
`Warning 1167, …, MeterAbsBi (Kotłownia,1d525782-02d4-82be-ffff60c50ef75c80)`) link to that block, and on the
deployed revision the block gets a `!` mark. Debug-Monitor lines count as "since the last program change" when they
are newer than it.
