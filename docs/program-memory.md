# Program memory from `/sys/Sizes.bin` (not built)

Issue #31 proposed estimating how much Miniserver memory a config needs, from `/sys/Sizes.bin`, and showing the
delta of a pending change in the Apply summary. On firmware 17 the file holds no sizes, so vislox does not do this.
This page records what was checked, so the idea is not picked up again on the same assumption.

## What was expected

[sarnau/Inside-The-Loxone-Miniserver](https://github.com/sarnau/Inside-The-Loxone-Miniserver) (`Code/loadSizes.py`)
describes `/sys/Sizes.bin` as an array of 32-bit little-endian values, one per object type, indexed by the type
number. The Miniserver was said to write it by creating each object type once and measuring it. With those sizes,
the program memory of a revision would be the sum, over its objects, of the size of each object's type.

## What the Miniserver has

Checked on 2026-11-06 on a Miniserver Gen 2 with firmware 17.2.10.21, read-only over FTP with lox's user.

- The file exists: `/sys/Sizes.bin`, 2200 bytes, written at the same time as the other files in `/sys` after the
  last firmware start (`IconLibrary.zip`, `Treesort.xml`, `sys_*.zip`).
- 2200 bytes is 550 little-endian `u32` values. 549 of them are `1000` (`e8 03 00 00`). Index 528 is `0`. No other
  value occurs.

The values are a constant, not measured sizes: the Document object (index 1) is "1000 bytes", the same as every
other type. A mapping from type indexes to the `Type` names in the `.Loxone` XML (or to the statistics' control-type
numbers) would not change the result, so it was not attempted. An estimate from this file would be
`1000 × number of objects`, which only counts objects and says nothing about memory.

## What is there instead

The real numbers come from the Miniserver itself: the Debug-Monitor's system packet carries the heap and its
maximum, and vislox shows them in the Miniserver chip's details ("Load (Debug-Monitor)"). This needs vislox
started with `--monitor` and a Debug-Monitor session started from the Log panel (see
[debug-monitor.md](debug-monitor.md)); without it the chip has no memory figure.

It describes the program that runs now, not a pending change. A change's cost shows only after Apply.

## If it comes back

Older firmware may still write measured sizes. Before building on the file again, read it from the Miniserver
in question and check that the values differ between types. A file of constants means there is nothing to map.
