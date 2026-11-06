# The Debug-Monitor stream (UDP 7777): findings

> Status: researched, not built. The Log panel shows `lox log` (see the README). This page records what a
> live Debug-Monitor would need, and why vislox does not do it yet.

## How the Miniserver sends it

The Miniserver does not let a listener register. It sends the monitor stream only when **Monitor Server** is set
in its settings (Loxone Config, Miniserver properties). It then asks that server what to send:
`GET http://<monitor server>/?<serial>`, and follows the XML it gets back
([sarnau/Inside-The-Loxone-Miniserver](https://github.com/sarnau/Inside-The-Loxone-Miniserver),
`LoxoneMiniserverNetworking.md`, "Loxone Monitor"):

```xml
<Log Version="10" LogKeep="true" NoIPchange="false">
  <LogMode>on</LogMode>
  <LogDest>/dev/udp/192.168.1.45/7777</LogDest>
  <LogLevelCommon>moreinfo</LogLevelCommon>
  <LogLevelSPS>moreinfo</LogLevelSPS>
  <LogLevelProtocol>off</LogLevelProtocol>
  <LogLevelBus>info</LogLevelBus>
  <LogLevelFilesystem>off</LogLevelFilesystem>
  <LogLevelNet>off</LogLevelNet>
</Log>
```

- Levels per area (Common, SPS, Protocol, Bus, Filesystem, Net): `off`, `severe`, `serious`, `warning`, `info`,
  `moreinfo`.
- How long: `LogStart`, `LogDay` (up to 7 days) or `LogUntil`.
- `LogDest` picks the address and port, so vislox could answer the request and receive the stream itself.

## The packets

From `Code/LoxoneMonitorServer.py` in the same repository (firmware 10.x, 2021):

- A frame starts with `0xFA1F` and a 16-bit size; byte 4 is an XOR checksum over bytes 5…size. `0x1F1F` is
  padding.
- A 28-byte header: parameter count, string ID, event counter, ms, Loxone time (seconds since 2009-01-01),
  source IP, used memory.
- String ID below `0xFFF0`: a log line. The ID points to a format string in `/sys/sys_ENG/ENG.xml` on the
  Miniserver (read over FTP), followed by typed parameters (string, buffer, int, IP, MAC, TCP flags).
- `0xFFFF`: system stats (CPU, heap, watchdog, LAN and Link errors). `0xFFFE`: the Miniserver's own I/O.
  `0xFFF0`–`0xFFFD`: the task list.

## Why vislox does not listen yet

- **It changes the Miniserver's settings.** Pointing Monitor Server at vislox is a config change on the
  Miniserver: it must be made in Loxone Config (or by uploading a config), reviewed like any other change, and
  turned off again afterwards. vislox only reads from the Miniserver outside Apply, so it does not do this on its
  own.
- **Nothing arrives without it.** Listening on UDP 7777 for 20 s next to a Miniserver on firmware 17.2 (Monitor
  Server not set) received no packets (6 Nov 2026).
- **The format is unverified on current firmware.** The sources date from firmware 10.x. The frame layout and
  the version field need checking against a Gen 2 Miniserver on 17.x, and the format strings come from a file
  read over FTP.
- **The network must let it through.** The vislox host's firewall must accept inbound UDP on the port, and it
  does not work over Remote Connect or from Docker's default bridge network.

## What it would add

If someone sets Monitor Server on purpose, vislox could answer the `GET /?<serial>` itself (choosing `LogDest` and
the levels), listen on that port while it runs, and add the lines to the Log panel live. The Bus level shows
device restarts and reconnects (useful around an Apply), and the system packet could feed the status chip (CPU,
heap) without polling.

## Linking messages to blocks

This works without the monitor: lines in the system log that `lox log` reads carry the UUID of the object that
raised them, e.g. `Warning 1167, …, MeterAbsBi (Kotłownia,1d525782-02d4-82be-ffff60c50ef75c80)`, where the UUID
is the meter block. The Log panel links those lines to their blocks. Whether a Logger block's lines reach the
system log was not checked (the house this was tested on has none in its log); if they do not, linking them needs
the monitor stream.
