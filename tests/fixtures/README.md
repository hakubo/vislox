# Test fixtures

`house.Loxone` is a small synthetic config written for vislox's tests. It is not a real Miniserver config.
It holds two pages, an input reference to a virtual input, an AND gate, a meter saved with hidden outputs,
a Divide block saved at the small block width, a note, and a wire from one page to the other. Its operating
modes are the holiday and vacation modes and a user mode (Party, `Mode="-1"`), with one calendar entry of each
kind the file stores: a fixed date, an Easter offset, a period, a yearly range, and one that switches the user
mode on.
