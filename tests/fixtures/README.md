# Test fixtures

`house.Loxone` is a small synthetic config written for vislox's tests. It is not a real Miniserver config.
It holds two pages, an input reference to a virtual input, an AND gate, a meter saved with hidden outputs,
a Divide block saved at the small block width, a note, and a wire from one page to the other. Its operating
modes are the holiday and vacation modes and a user mode (Party, `Mode="-1"`), with one calendar entry of each
kind the file stores: a fixed date, an Easter offset, a period, a yearly range, and one that switches the user
mode on.

`hardware.Loxone` is another small synthetic config, for the hardware review: a Miniserver with an analog
input of its own, a Tree Extension with a Touch Tree (audible acknowledgement on, and a temperature input) and
a Room Comfort Sensor Tree on its left branch, an AI Extension with one voltage input, and a DI Extension with
one digital input. Its serials are made up.

`memory.Loxone` is a third small synthetic config, for memory flags (#104): two pages, three memory flags and a
reference to each of two of them. *Anwesend* is written on its page and read on the other through an input
reference; *Alarm* is written from the other page by a wire and read nowhere; *Hilfsmerker* is written by an AND
and by an output reference on the other page, and read by a NOT on its page (so it shows as two tags).
