// Browser test for editing in vislox (values, new blocks, moves, wires): a lox.toml project with nothing
// pending, and a stand-in lox whose `logic compile` writes what vislox's files say into out.Loxone. An edit
// must show as the pending revision, marked as not deployed, with Revert and Discard, and nothing
// may ever be deployed: the stand-in logs every call and fails on anything but `logic compile`.
//
//   VISLOX_BIN=target/release/vislox node tests/ui/edit.mjs
//
// Nothing here talks to a Miniserver: vislox runs with --no-live.
import { chromium } from "playwright";
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, writeFileSync, chmodSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const here = dirname(fileURLToPath(import.meta.url));
const bin = process.env.VISLOX_BIN || join(here, "../../target/release/vislox");
const fixture = join(here, "../fixtures/house.Loxone");

const DIV = "10000006-0000-0060-ffff504f94000001";
const MONO = "1000000a-0000-00a0-ffff504f94000001";
const OG = "10000008-0000-0080-ffff504f94000001";
const EG = "10000002-0000-0020-ffff504f94000001";
const AND = "10000004-0000-0040-ffff504f94000001", METER = "10000005-0000-0050-ffff504f94000001";
// A block the project's module manages (in the lock as `flur`), drawn on the upper floor.
const FLUR = "1000000b-0000-00b0-ffff504f94000001";
// A page of more managed blocks than one request takes ops (200).
const LAGER = "20000000-0000-0000-ffff504f94000001", LAGER_N = 210;
const lagerUuid = (i) => `2${String(i).padStart(7, "0")}-0000-0001-ffff504f94000001`;
const lagerAt = (i) => { const px = 960 + 1920 * (i % 15), py = 960 + 960 * Math.floor(i / 15); return { px, py, px2: px + 1344, py2: py + 696 }; };
// An NFC Code Touch on a page of its own (Zugang), for the users lox logic lint says it opens for none of (#66).
const ZUGANG = "40000000-0000-0000-ffff504f94000001", TOUCH = "40000001-0000-0000-ffff504f94000001";
// Users, a group and two access schedules (#76): Anna may open Haustür always (and operates it in the app: ACT=),
// Ben is in Bewohner; Garage, a second Code Touch, opens for nobody. Anna has an NFC tag and a code, never shown.
const ANNA = "40000011-0000-0000-ffff504f94000001", BEN = "40000012-0000-0000-ffff504f94000001", BEWOHNER = "40000013-0000-0000-ffff504f94000001";
const ALWAYS = "40000016-0000-0000-ffff504f94000001", WERKTAGS = "40000017-0000-0000-ffff504f94000001", GARAGE = "40000002-0000-0000-ffff504f94000001";
// Push notifications (#77): Frostwarnung, which Anna gets, and Regen, which nobody does, in Messages; Ben is in Full
// access, so he gets every notification (inherited). Frostwarnung is drawn on the page Meldungen with its message.
const FULL = "40000018-0000-0000-ffff504f94000001", FROST = "40000021-0000-0000-ffff504f94000001", REGEN = "40000022-0000-0000-ffff504f94000001";
const MELDUNGEN = "40000030-0000-0000-ffff504f94000001", WARN = "40000031-0000-0000-ffff504f94000001";
// Block settings (#79): a page Zeiten that Loxone Config drew, with a PulseAt firing at 18:58, a program block
// (Code16) and a Monoflop whose Time is 1800 s.
const ZEITEN = "50000000-0000-0000-ffff504f94000001", PULSE = "50000001-0000-0000-ffff504f94000001";
const HUE = "50000002-0000-0000-ffff504f94000001", NACH = "50000003-0000-0000-ffff504f94000001";
// Automatic Designer rules (#87): a page Regeln with a push button the app shows as a switch (Pumpe), the rule Pumpe
// Ein (active, weekdays at 09:00: Pumpe on, as the app writes it) and Ansage, whose speech output Lox Logic cannot
// write; their AutoPilot container.
const REGELN = "a0000000-0000-0000-ffff504f94000001", R_PUMP = "a0000001-0000-0000-ffff504f94000001";
const R_EIN = "a0000002-0000-0000-ffff504f94000001", R_ODD = "a0000003-0000-0000-ffff504f94000001", AUTOPILOT = "a0000004-0000-0000-ffff504f94000001";
const apc = (o) => JSON.stringify(o).replace(/"/g, "&quot;");
const regelnXml = `\t\t<C Type="AutoPilot" V="178" U="${AUTOPILOT}" Title="Regeln"/>
\t\t<C Type="Page" V="178" U="${REGELN}" Title="Regeln" WF="16384">
\t\t\t<C Type="PushButton" V="178" U="${R_PUMP}" Title="Brunnenpumpe" Px="960" Py="960" Px2="2304" Py2="1656" Cl="141,255,112" Nio="2" WF="16384" SpStates="a0000001-0000-0013-ffff504f94000001,70000001-0000-0014-ffff504f94000001,70000001-0000-0015-ffff504f94000001">
\t\t\t\t<Co K="Reset" U="a0000001-0000-0001-ffff504f94000001"/>
\t\t\t\t<Co K="Q" U="a0000001-0000-0002-ffff504f94000001"/>
\t\t\t\t<IoData Visu="true" Pr="${"10000041-0000-0000-ffff504f94000001"}"/>
\t\t\t\t<Display Type="1"/>
\t\t\t</C>
\t\t\t<C Type="AutopilotRule" V="178" U="${R_EIN}" Title="Pumpe Ein" Px="960" Py="2880" Px2="2304" Py2="3576" Cl="141,255,112" Nio="1" WF="16384" APC="${apc({ type: 1, id: R_EIN, active: true, activeIfValid: true, name: "Pumpe Ein", description: "", operator: 3, invalid: false, eventGroups: [{ operator: 1, events: [{ type: 3, descID: 270001, id: 270, value: 9999999999, operator: 1, forceEdge: false, arrValues: [0, 1, 2, 3, 4] }, { type: 5, descID: 272001, id: 272, value: 540, operator: 1, forceEdge: false }] }], actions: [{ type: 0, descID: 342101, id: R_PUMP, command: "on" }] })}" STA="1" ref="${AUTOPILOT}">
\t\t\t\t<Co K="Q" U="a0000002-0000-0001-ffff504f94000001"/>
\t\t\t</C>
\t\t\t<C Type="AutopilotRule" V="178" U="${R_ODD}" Title="Ansage" Px="960" Py="4800" Px2="2304" Py2="5496" Cl="141,255,112" Nio="1" WF="16384" APC="${apc({ type: 1, id: R_ODD, active: true, activeIfValid: true, name: "Ansage", description: "", operator: 3, invalid: false, eventGroups: [{ operator: 1, events: [{ type: 5, descID: 272001, id: 272, value: 600, operator: 1, forceEdge: false }] }], actions: [{ type: 9, descID: 999999, id: 0, command: "say" }] })}" STA="1" ref="${AUTOPILOT}">
\t\t\t\t<Co K="Q" U="a0000003-0000-0001-ffff504f94000001"/>
\t\t\t</C>
\t\t</C>
`;
const zeitenXml = `\t\t<C Type="Page" V="178" U="${ZEITEN}" Title="Zeiten" WF="16384">
\t\t\t<C Type="PulseAt" V="178" U="${PULSE}" Title="Abends" Px="960" Py="960" Px2="2304" Py2="1656" Cl="141,255,112" Nio="1" WF="16384" Typ="0" Sec="68280">
\t\t\t\t<Co K="Q" U="50000001-0000-0001-ffff504f94000001"/>
\t\t\t</C>
\t\t\t<C Type="Code16" V="178" U="${HUE}" Title="Hue" Px="960" Py="2880" Px2="2304" Py2="3576" Cl="141,255,112" Nio="1" WF="16384" Code="setoutput(0, 1);" Task="1">
\t\t\t\t<Co K="AQ1" U="50000002-0000-0001-ffff504f94000001"/>
\t\t\t</C>
\t\t\t<C Type="Monoflop" V="178" U="${NACH}" Title="Nachlauf Zeit" Px="960" Py="4800" Px2="2304" Py2="5496" Cl="141,255,112" Nio="4" WF="16384">
\t\t\t\t<Co K="InputTrigger" U="50000003-0000-0001-ffff504f94000001"/>
\t\t\t\t<Co K="Time" Def="1800" U="50000003-0000-0002-ffff504f94000001"/>
\t\t\t\t<Co K="Q" U="50000003-0000-0003-ffff504f94000001"/>
\t\t\t</C>
\t\t</C>
`;
// Constants (#81): a page Konstanten that Loxone Config drew, with three Monoflops. The module's own file binds K1's
// Time to its constant `nachlauf` (30min); K2 runs 30min and K3 1min, values of their own.
const KONST = "60000000-0000-0000-ffff504f94000001", K1 = "60000001-0000-0000-ffff504f94000001";
const K2 = "60000002-0000-0000-ffff504f94000001", K3 = "60000003-0000-0000-ffff504f94000001";
// And two Monoflops the module manages (pages/zeiten.loxl), for a default (#81): Kurz takes Loxone Config's 1min,
// Lang sets its own 1h.
const K4 = "60000004-0000-0000-ffff504f94000001", K5 = "60000005-0000-0000-ffff504f94000001";
const monoXml = (u, title, y, time) => `\t\t\t<C Type="Monoflop" V="178" U="${u}" Title="${title}" Px="960" Py="${y}" Px2="2304" Py2="${y + 696}" Cl="141,255,112" Nio="4" WF="16384">
\t\t\t\t<Co K="InputTrigger" U="${u.slice(0, 9)}0000-0001-ffff504f94000001"/>
\t\t\t\t<Co K="Time" Def="${time}" U="${u.slice(0, 9)}0000-0002-ffff504f94000001"/>
\t\t\t\t<Co K="Q" U="${u.slice(0, 9)}0000-0003-ffff504f94000001"/>
\t\t\t</C>
`;
const konstXml = `\t\t<C Type="Page" V="178" U="${KONST}" Title="Konstanten" WF="16384">
${monoXml(K1, "Treppe", 960, 1800)}${monoXml(K2, "Keller", 2880, 1800)}${monoXml(K3, "Garten", 4800, 60)}${monoXml(K4, "Kurz", 6720, 60)}${monoXml(K5, "Lang", 8640, 3600)}\t\t</C>
`;
const usersXml = `\t\t<C Type="LoxCaption" V="178" U="40000010-0000-0000-ffff504f94000001" Title="Users and Permissions" CaptionType="20" SubType="20">
\t\t\t<C Type="User" V="178" U="${ANNA}" Title="Anna" UsR="-1" NFCArr="0A1B2C3D4E5F" CodeArr="00112233">
\t\t\t\t<PH NUM="2">
\t\t\t\t\t<PRM REF="${TOUCH}" TIM="${ALWAYS}" ACT="3"/>
\t\t\t\t\t<PRM REF="${FROST}" ACT="3"/>
\t\t\t\t</PH>
\t\t\t</C>
\t\t\t<C Type="User" V="178" U="${BEN}" Title="Ben" UsR="129">
\t\t\t\t<PH NUM="0">
\t\t\t\t</PH>
\t\t\t</C>
\t\t\t<C Type="UserGroup" V="178" U="${BEWOHNER}" Title="Bewohner" Rights="129" User="${BEN}" UserGroups="">
\t\t\t\t<PH NUM="0">
\t\t\t\t</PH>
\t\t\t</C>
\t\t\t<C Type="UserGroup" V="178" U="${FULL}" Title="Full access" Rights="-1" UgType="4" User="${BEN}" UserGroups=""/>
\t\t\t<C Type="SwitchingTimer" V="178" U="${ALWAYS}" Title="Always" M="3" FIX="1"/>
\t\t\t<C Type="SwitchingTimer" V="178" U="${WERKTAGS}" Title="Werktags" M="3"/>
\t\t</C>
\t\t<C Type="LoxCaption" V="178" U="40000020-0000-0000-ffff504f94000001" Title="Messages" CaptionType="9" SubType="9">
\t\t\t<C Type="Notification" V="178" U="${FROST}" Title="Frostwarnung" WF="16400" Entries="10"/>
\t\t\t<C Type="Notification" V="178" U="${REGEN}" Title="Regen" WF="16384" Entries="10"/>
\t\t</C>
`;
// Expressions (#82): a page Ausdruck, where the module's own file feeds Markise's I2 from the expression
// `licht.Q and teiler.AQ >= 28`: lox made an And (EXA) and a GreaterEqual (EXG) for it, expression-owned in the lock.
// Pumpe's inputs are free: vislox gives one an expression of its own.
const AUSDRUCK = "80000000-0000-0000-ffff504f94000001", MARK = "80000001-0000-0000-ffff504f94000001";
const EXA = "80000002-0000-0000-ffff504f94000001", EXG = "80000003-0000-0000-ffff504f94000001", PUMPE = "80000004-0000-0000-ffff504f94000001";
const gateXml = (ty, u, title, y, ins) => `\t\t\t<C Type="${ty}" V="178" U="${u}" Title="${title}" Px="960" Py="${y}" Px2="2304" Py2="${y + 696}" Cl="141,255,112" Nio="3" WF="16384">
\t\t\t\t<Co K="I1" U="${u.slice(0, 14)}0001-${u.slice(19)}"/>
\t\t\t\t<Co K="I2" U="${u.slice(0, 14)}0002-${u.slice(19)}"${ins ? `>\n\t\t\t\t\t<In Input="${ins}"/>\n\t\t\t\t</Co>` : "/>"}
\t\t\t\t<Co K="Q" U="${u.slice(0, 14)}0003-${u.slice(19)}"/>
\t\t\t</C>
`;
// Template instances (#83): a page Vorlagen, where the module's own file calls its template Paar as `nord`: lox made
// nord's one block, `nord_g` in the lock (NORD_G).
const VORLAGEN = "90000000-0000-0000-ffff504f94000001", NORD_G = "90000001-0000-0000-ffff504f94000001";
const vorlagenXml = `\t\t<C Type="Page" V="178" U="${VORLAGEN}" Title="Vorlagen" WF="16384">
${gateXml("GreaterEqual", NORD_G, "Nord", 960, null)}\t\t</C>
`;
const ausdruckXml = `\t\t<C Type="Page" V="178" U="${AUSDRUCK}" Title="Ausdruck" WF="16384">
${gateXml("GreaterEqual", EXG, "teiler.AQ &gt;= 28", 960, null)}${gateXml("And", EXA, "licht.Q and teiler.AQ &gt;= 28", 2880, `${EXG.slice(0, 14)}0003-${EXG.slice(19)}`)}${gateXml("And", MARK, "Markise", 4800, `${EXA.slice(0, 14)}0003-${EXA.slice(19)}`)}${gateXml("And", PUMPE, "Pumpe", 6720, null)}\t\t</C>
`;
// Objects of the object tree (#84): the house's Virtual Inputs folder (VI_F), a Virtual Outputs folder and a
// Netzwerkperipherie folder with a Modbus server Loxone Config made (WP, a made-up address).
const VI_F = "10000000-0000-0002-ffff504f94000001", NET_F = "7e000001-0000-0000-ffff504f94000001", WP = "7e000002-0000-0000-ffff504f94000001";
const IMPORT_HTTP = "7d000002-0000-0000-ffff504f94000001", IMPORT_CMD = "7d000001-0000-0000-ffff504f94000001";
const importedHttpXml = `\t\t\t<C Type="VirtualHttpIn" V="178" U="${IMPORT_HTTP}" Title="Imported HTTP" Address="http://192.0.2.30/status" Poll="60">
\t\t\t\t<C Type="VirtualHttpInCmd" V="178" U="${IMPORT_CMD}" Title="Imported Command" Check="value">
\t\t\t\t</C>
\t\t\t</C>
`;
const periphXml = `\t\t<C Type="VirtualOutCaption" V="178" U="7e000000-0000-0000-ffff504f94000001" Title="Virtuelle Ausgänge">\n\t\t</C>\n` +
  `\t\t<C Type="WeatherCaption" V="178" U="${NET_F}" Title="Netzwerkperipherie">\n` +
  `\t\t\t<C Type="ModbusServer" V="178" U="${WP}" Title="Wärmepumpe" Cl="0,0,0" WF="16384" Address="192.0.2.12:502" Timeout="2000">\n\t\t\t</C>\n\t\t</C>\n`;
// Tree hardware (#85): a Tree Extension (HW_T) with a Touch Tree (HW_D) whose temperature (HW_A) records no
// statistics and whose first touch zone (HW_S) records them (StatsType 7).
// Operating modes and calendar entries (#86): the house's Betriebsmodi folder (with Party, a mode Loxone Config made),
// its Betriebszeiten folder, and Heiligabend, an entry Loxone Config made. PARTY is declared below.
const MODE_F = "10000010-0000-0100-ffff504f94000001", CAL_F = "10000012-0000-0120-ffff504f94000001";
const HEILIGABEND = "10000013-0000-0130-ffff504f94000001";
const HW_T = "7f000001-0000-0000-ffff504f94000001", HW_D = "7f000002-0000-0000-ffff504f94000001";
const HW_A = "7f000003-0000-0000-ffff504f94000001", HW_S = "7f000004-0000-0000-ffff504f94000001";
const hwXml = `\t\t<C Type="LoxTree" V="178" U="${HW_T}" Title="Tree Extension" WF="16384" Serial="13000001">
\t\t\t<C Type="TreeDevice" TId="TouchTree" V="178" U="${HW_D}" Title="Touch Bad" WF="16384" Serial="B0B0B001" SubType="32771" TreeType="32771">
\t\t\t\t<C Type="TreeAsensor" IName="AI1" V="178" U="${HW_A}" Title="Temperatur Bad" Nio="2" WF="16400" Analog="true"><Co K="AQ" U="7f000003-0000-0001-ffff504f94000001"/></C>
\t\t\t\t<C Type="TreeSensor" IName="I1" V="178" U="${HW_S}" Title="Taste Bad" Nio="2" WF="16384" StatsType="7"><Co K="Q" U="7f000004-0000-0001-ffff504f94000001"/></C>
\t\t\t</C>
\t\t</C>
`;
const KITCHEN = "30000a01-0000-0000-ffff504f94000001", BATH = "30000a02-0000-0000-ffff504f94000001";
// Two analog Daytimers on the upper floor (#73): Uhr, on Mondays 06:30–08:00 and in the Party mode 18:00–23:00,
// and Heizung, with no switching times; and Licht, a digital one.
const UHR = "10000020-0000-0200-ffff504f94000001", HEIZ = "10000021-0000-0210-ffff504f94000001", LICHT = "10000022-0000-0220-ffff504f94000001";
const PARTY = "10000011-0000-0110-ffff504f94000001", MON = "00000000-0000-0004-1500000000000000";
const dayU = (n) => `00000000-0000-${n.toString(16).padStart(4, "0")}-1500000000000000`;
const timerXml = (u, title, y, modes, entries, analog = true) => `\t\t\t<C Type="DayTimer" V="178" U="${u}" Title="${title}" Px="960" Py="${y}" Px2="3648" Py2="${y + 888}" Nio="2" WF="16384"${analog ? ' Analog="true" DefValue="18"' : ""}${modes ? ` Modes="${modes}"` : ""}>
\t\t\t\t<Co K="Mode" U="${u.slice(0, 19)}1-00ff504f94000001"/>
\t\t\t\t<Co K="AQ" U="${u.slice(0, 19)}2-01ff504f94000001"/>
${entries.map((e) => `\t\t\t\t${e}\n`).join("")}\t\t\t</C>
`;
// Two lighting controllers on the upper floor (#74): Wohnzimmer, whose T5/1 (Sel1) a button wires to "Lesen", and
// Diele. AQ1 is a switch (Deckenlicht), AQ2 a dimmer without an actuator, AQ3 tunable white, the rest switches; the
// moods are "Viel Licht" (777), "Lesen" (1), "Kino" (2) and the off mood "Aus" (778), last.
const LC_W = "10000040-0000-0400-ffff504f94000001", LC_F = "10000041-0000-0410-ffff504f94000001";
const TW = (b, k) => 6 * 2 ** 28 + 200000000 + 10000 * b + k;
const lcXml = (u, title, y, button) => {
  const mood = (name, sid, cid, flag, q, rest) => `\t\t\t\t\t<LightsceneC Name="${name}" UUID="${u.slice(0, 33)}${cid}"${flag} Outputs="18" SID="${sid}" CID="${cid}" ${[...q, ...Array(15).fill(rest)].map((v, i) => `Q${i + 1}="${v}"`).join(" ")}/>\n`;
  const co = (k, n, from) => `\t\t\t\t<Co K="${k}" U="${u.slice(0, 17)}${n}-00ff504f94000001"${from ? `>\n\t\t\t\t\t<In Input="${from}"/>\n\t\t\t\t</Co>` : "/>"}\n`;
  return `\t\t\t<C Type="LightController2" V="178" U="${u}" Title="${title}" Px="12480" Py="${y}" Px2="15168" Py2="${y + 2232}" Nio="5" NameAI0="Deckenlicht" PresM0="65535">\n` +
    co("Sel1", 1, button ? "10000004-0000-0043-02ff504f94000001" : null) + co("Sel2", 2) + co("AQ1", 3) + co("AQ2", 4) + co("AQ3", 5) +
    `\t\t\t\t<LightscenesC FC="0040FFFF" Outputs="18" Num="4">\n` +
    mood("Viel Licht", 777, 22, ' Move="true"', [1, 2 ** 28 + 100, TW(100, 2700)], 1) +
    mood("Lesen", 1, 12, "", [1, 2 ** 28 + 80, 6 * 2 ** 28], 0) +
    mood("Kino", 2, 13, "", [0, 2 ** 28, TW(20, 2700)], 0) +
    mood("Aus", 778, 11, ' ReadOnly="true"', [0, 2 ** 28, 6 * 2 ** 28], 0) +
    `\t\t\t\t</LightscenesC>\n\t\t\t\t<LSConfig OpM="778" ScMv="777" ScAc="1"/>\n\t\t\t</C>\n`;
};
// A CentralLight on the upper floor (#75), driving Wohnzimmer; and an alarm whose members stay with Loxone Config.
const CENTRAL = "10000050-0000-0500-ffff504f94000001", ALARM = "10000053-0000-0530-ffff504f94000001";
const centralXml = `\t\t\t<C Type="CentralLight" V="178" U="${CENTRAL}" Title="Alle Lichter" Px="17280" Py="4800" Px2="18624" Py2="5760" Nio="0" rec="${LC_W}" SpStates="0"/>\n` +
  `\t\t\t<C Type="Alarm" V="178" U="${ALARM}" Title="Alarmanlage" Px="17280" Py="7680" Px2="18624" Py2="8640" Nio="0" Entries="40" CentralOut="1" Objects="${LC_F}"/>\n`;
// A Touch Pure Flex, Flex Küche (#78), with tiles 2, 3 and 5: tile 3's click toggles Nord, tile 5's click toggles every
// linked shade (lox logic lint says so: tile-broadcast). On the page Beschattung, Nord's OutputAPI is wired to the
// device's API connector, Süd's through an OutputRef of it; Ost is linked to nothing.
const FLEX = "70000001-0000-0000-ffff504f94000001", FLEX_API = "70000002-0000-0000-ffff504f94000001", FLEX_I = "70000002-0000-0001-ffff504f94000001";
const BESCH = "70000010-0000-0000-ffff504f94000001", NORD = "70000011-0000-0000-ffff504f94000001", SUED = "70000012-0000-0000-ffff504f94000001";
const OST = "70000013-0000-0000-ffff504f94000001", API_REF = "70000014-0000-0000-ffff504f94000001";
const coU = (u, n) => `${u.slice(0, 14)}000${n}-${u.slice(19)}`;
const flexXml = `\t\t<C Type="LoxCaption" V="178" U="70000000-0000-0000-ffff504f94000001" Title="Tree" CaptionType="2">
\t\t\t<C Type="TreeDevice" TId="TpfTree" V="178" U="${FLEX}" Title="Flex Küche" WF="16384" Serial="B0500001" SubType="32799" TreeType="32799" cfgId="1">
\t\t\t\t<C Type="ApiActor" V="178" U="${FLEX_API}" Title="API Connector">
\t\t\t\t\t<Co K="I" U="${FLEX_I}">
\t\t\t\t\t\t<In Input="${coU(API_REF, 2)}"/>
\t\t\t\t\t</Co>
\t\t\t\t</C>
\t\t\t\t<btns>
\t\t\t\t\t<btn i="2" x="6" y="7" h="13" w="15">
\t\t\t\t\t\t<f1></f1>
\t\t\t\t\t\t<f2 bT="1"/>
\t\t\t\t\t\t<f3 bT="2"/>
\t\t\t\t\t</btn>
\t\t\t\t\t<btn ci="1" i="3" x="37" y="7" h="13" w="15">
\t\t\t\t\t\t<f1 api="SET(As;Tg;pulse)" cT="=ECHO(As)" ctrls="${NORD}"/>
\t\t\t\t\t\t<f2 bT="1"/>
\t\t\t\t\t\t<f3 bT="2"/>
\t\t\t\t\t</btn>
\t\t\t\t\t<btn ci="2" i="5" x="6" y="37" h="13" w="15">
\t\t\t\t\t\t<f1 api="SET(As;Tg;pulse)"/>
\t\t\t\t\t\t<f2 bT="1"/>
\t\t\t\t\t\t<f3 bT="2"/>
\t\t\t\t\t</btn>
\t\t\t\t</btns>
\t\t\t</C>
\t\t</C>
`;
const jalXml = (u, title, y, input) => `\t\t\t<C Type="AutoJalousie" V="178" U="${u}" Title="${title}" Px="960" Py="${y}" Px2="2304" Py2="${y + 696}" Cl="141,255,112" Nio="1" WF="16384">
\t\t\t\t<Co K="OutputAPI" U="${coU(u, 1)}"${input ? `>\n\t\t\t\t\t<In Input="${input}"/>\n\t\t\t\t</Co>` : "/>"}
\t\t\t</C>
`;
const beschXml = `\t\t<C Type="Page" V="178" U="${BESCH}" Title="Beschattung" WF="16384">
${jalXml(NORD, "Nord", 960, FLEX_I)}${jalXml(SUED, "Süd", 2880, null)}${jalXml(OST, "Ost", 4800, null)}\t\t\t<C Type="OutputRef" V="178" U="${API_REF}" Title="API Connector" Px="9600" Py="960" Px2="10944" Py2="1656" Cl="0,0,0" Nio="2" Ref="${FLEX_API}">
\t\t\t\t<Co K="AI" U="${coU(API_REF, 1)}">
\t\t\t\t\t<In Input="${coU(SUED, 1)}"/>
\t\t\t\t</Co>
\t\t\t\t<Co K="AQ" U="${coU(API_REF, 2)}"/>
\t\t\t</C>
\t\t</C>
`;
const root = mkdtempSync(join(tmpdir(), "vislox-edit-"));
const dir = join(root, "project");
mkdirSync(join(dir, "pages"), { recursive: true });
const git = (...a) => execFileSync("git", ["-C", dir, "-c", "user.name=t", "-c", "user.email=t@example.com", ...a]);
// The deployed config, with a Monoflop on the upper floor (its Time has a documented range: 0 or more).
const base0 = readFileSync(fixture, "utf8").replace('<Co K="Q" U="10000009-0000-0093-02ff504f94000001"/>\n\t\t\t</C>', `<Co K="Q" U="10000009-0000-0093-02ff504f94000001"/>
\t\t\t</C>
\t\t\t<C Type="Monoflop" V="178" U="${MONO}" Title="Treppe" Px="960" Py="2880" Px2="2304" Py2="3576" Cl="141,255,112" Nio="4" WF="16384">
\t\t\t\t<Co K="InputTrigger" U="1000000a-0000-00a1-00ff504f94000001"/>
\t\t\t\t<Co K="Time" Def="2" U="1000000a-0000-00a2-01ff504f94000001"/>
\t\t\t\t<Co K="Q" U="1000000a-0000-00a3-02ff504f94000001"/>
\t\t\t</C>
\t\t\t<C Type="And" V="178" U="${FLUR}" Title="Flur" Px="7104" Py="2880" Px2="8448" Py2="3576" Cl="141,255,112" Nio="3" WF="16384">
\t\t\t\t<Co K="I1" U="1000000b-0000-00b1-00ff504f94000001"/>
\t\t\t\t<Co K="I2" U="1000000b-0000-00b2-01ff504f94000001"/>
\t\t\t\t<Co K="Q" U="1000000b-0000-00b3-02ff504f94000001"/>
\t\t\t</C>`)
  .replace(`\t\t\t<C Type="And" V="178" U="${FLUR}"`, timerXml(UHR, "Uhr", 4800, `${MON},${PARTY}`, ['<Entry From="390" To="480" V="21"/>', '<Entry From="1080" To="1380" Ix="1" V="22.5"/>']) +
    timerXml(HEIZ, "Heizung", 6720, "", []) + timerXml(LICHT, "Licht", 8640, "", [], false) +
    lcXml(LC_W, "Wohnzimmer", 4800, true) + lcXml(LC_F, "Diele", 7680, false) + centralXml + `\t\t\t<C Type="And" V="178" U="${FLUR}"`)
  // Two rooms; the Divide block is shown in the app, in the kitchen.
  .replace('\t\t<C Type="Page" V="178" U="10000002', `${usersXml}\t\t<C Type="Place" V="178" U="${KITCHEN}" Title="Küche"/>
\t\t<C Type="Place" V="178" U="${BATH}" Title="Bad"/>
\t\t<C Type="Page" V="178" U="10000002`)
  .replace('<Co K="Input1" Nc="1" U="10000006', `<IoData Visu="true" Pr="${KITCHEN}"/>\n\t\t\t\t<Co K="Input1" Nc="1" U="10000006`)
  .replace("\t\t</C>\n\t</C>\n</ControlList>", `\t\t</C>\n\t\t<C Type="Page" V="178" U="${LAGER}" Title="Lager" WF="16384">\n` +
    Array.from({ length: LAGER_N }, (_, i) => { const L = lagerAt(i); return `\t\t\t<C Type="And" V="178" U="${lagerUuid(i)}" Title="Lager ${i}" Px="${L.px}" Py="${L.py}" Px2="${L.px2}" Py2="${L.py2}" Cl="141,255,112" Nio="3" WF="16384"/>\n`; }).join("") +
    "\t\t</C>\n" +
    `\t\t<C Type="Page" V="178" U="${ZUGANG}" Title="Zugang" WF="16384">\n` +
    `\t\t\t<C Type="NfcCodeTouch" V="178" U="${TOUCH}" Title="Haustür" Px="960" Py="960" Px2="2304" Py2="1656" Cl="141,255,112" Nio="0" WF="16384"/>\n` +
    `\t\t\t<C Type="NfcCodeTouch" V="178" U="${GARAGE}" Title="Garage" Px="960" Py="2880" Px2="2304" Py2="3576" Cl="141,255,112" Nio="0" WF="16384"/>\n` +
    "\t\t</C>\n" +
    `\t\t<C Type="Page" V="178" U="${MELDUNGEN}" Title="Meldungen" WF="16384">\n` +
    `\t\t\t<C Type="OutputRefLM" V="178" U="${WARN}" Title="Frostwarnung" Px="960" Py="960" Px2="2304" Py2="1656" Cl="0,0,0" Nio="2" Ref="${FROST}"><Co K="AI" U="40000031-0000-0001-ffff504f94000001"/><Co K="AQ" U="40000031-0000-0002-ffff504f94000001"/><LoggerMailer RefNotification="${FROST}" On="Frost im Garten" MinimumTime="43200000" NL="1"/></C>\n` +
    "\t\t</C>\n" + regelnXml + zeitenXml + konstXml + ausdruckXml + vorlagenXml + "\t</C>\n</ControlList>");
const base = base0.replace('<C Type="VirtualIn"', `${importedHttpXml}<C Type="VirtualIn"`).replace(`\t\t<C Type="Place" V="178" U="${KITCHEN}"`, `${flexXml}\t\t<C Type="Place" V="178" U="${KITCHEN}"`)
  .replace(`\t\t<C Type="Page" V="178" U="${MELDUNGEN}"`, `${beschXml}\t\t<C Type="Page" V="178" U="${MELDUNGEN}"`)
  .replace('\t\t<C Type="ModeCaption"', `${periphXml}${hwXml}\t\t<C Type="ModeCaption"`);
assert.ok(base.includes(WP), "the Modbus server is in the base");
assert.ok(base.includes(HW_A), "the Tree Extension is in the base");
assert.ok(base.includes(FLEX) && base.includes(OST), "the Touch Pure Flex is in the base");
assert.ok(base.includes(lagerUuid(LAGER_N - 1)), "the Lager page is in the base");
writeFileSync(join(dir, "base.Loxone"), base);
writeFileSync(join(dir, "out.Loxone"), base);
writeFileSync(join(dir, "lox.toml"), 'base = "base.Loxone"\nmodule = "pages"\nout = "out.Loxone"\n');
writeFileSync(join(dir, "pages", "README"), "the module\n");
// The module's own source (the stand-in lox reads only vislox's files): `flur` is declared on line 3.
writeFileSync(join(dir, "pages", "flur.loxl"), '# The module\'s own source.\npage "Obergeschoss"\nflur = And("Flur")\n');
// The module's own constant (#81), and K1's Time bound to it on line 3.
writeFileSync(join(dir, "pages", "konst.loxl"), `let nachlauf = 30min\nextern k1 = Monoflop(uuid: "${K1}")\nk1.Time = nachlauf\n`);
writeFileSync(join(dir, "pages", "zeiten.loxl"), 'page "Konstanten"\nkurz = Monoflop("Kurz")\nlang = Monoflop("Lang", Time: 1h)\n');
// The module's own expression (#82), on line 5.
writeFileSync(join(dir, "pages", "ausdruck.loxl"), `extern markise = And(uuid: "${MARK}")\nextern licht = Or(uuid: "10000009-0000-0090-ffff504f94000001")\nextern teiler = Div(uuid: "${DIV}")\n\nmarkise.In2 <- licht.Q and teiler.AQ >= 28\n`);
// The module's own template and its instance (#83): the template on line 2, the call on line 6.
writeFileSync(join(dir, "pages", "vorlage.loxl"), 'page "Vorlagen"\ntemplate Paar(label: string = "P", quelle: input, grenze: number = 28) -> (Q: g.Q)\n\tg = GreaterEqual("{label}", In1: quelle, In2: grenze)\nend\n\nnord = Paar("Nord", quelle: teiler.AQ)\n');
const FLUR_AT = { px: 7104, py: 2880, px2: 8448, py2: 3576 };
const lockObjects = { flur: { uuid: FLUR, type: "And", page_uuid: OG, layout: FLUR_AT } };
for (let i = 0; i < LAGER_N; i++) lockObjects[`lager_${i}`] = { uuid: lagerUuid(i), type: "And", page_uuid: LAGER, layout: lagerAt(i) };
lockObjects.markise_i2__ge1 = { uuid: EXG, type: "GreaterEqual", page_uuid: AUSDRUCK, layout: { px: 960, py: 960, px2: 2304, py2: 1656 }, expr_owned: true };
lockObjects.kurz = { uuid: K4, type: "Monoflop", page_uuid: KONST, layout: { px: 960, py: 6720, px2: 2304, py2: 7416 } };
lockObjects.lang = { uuid: K5, type: "Monoflop", page_uuid: KONST, layout: { px: 960, py: 8640, px2: 2304, py2: 9336 } };
lockObjects.nord_g = { uuid: NORD_G, type: "GreaterEqual", page_uuid: VORLAGEN, layout: { px: 960, py: 960, px2: 2304, py2: 1656 } };
lockObjects.markise_i2__and1 = { uuid: EXA, type: "And", page_uuid: AUSDRUCK, layout: { px: 960, py: 2880, px2: 2304, py2: 3576 }, expr_owned: true };
writeFileSync(join(dir, "logic.lock.json"), JSON.stringify({ objects: lockObjects }));
// Compiled after the module's files: nothing is uncompiled.
writeFileSync(join(dir, "out.Loxone"), base);
git("init", "-q");
git("add", "-A");
git("commit", "-q", "-m", "Deployed");

// The stand-in lox. `logic compile` reads vislox's claims (extern …(uuid: "…"), x.title = "…",
// x.Port = n) and writes them into a copy of the base, as lox would. It draws every block of the lock
// where its `layout` says, and `--relayout <page>` lines that page's blocks up along the top. It takes
// the title of a block the module's own files declare (`flur = And("Flur")`). `logic check --json`
// finds an error on each line of the module's own files that says BROKEN; `logic fmt --check` finds
// nothing to format.
const loxDir = mkdtempSync(join(tmpdir(), "vislox-lox-"));
const calls = join(loxDir, "calls.log");
const fakeLox = join(loxDir, "lox");
writeFileSync(fakeLox, `#!/usr/bin/env node
const fs = require("fs");
fs.appendFileSync(${JSON.stringify(calls)}, process.argv.slice(2).join(" ") + "\\n");
const own = () => fs.readdirSync("pages").filter((f) => f.endsWith(".loxl")).map((f) => ["pages/" + f, fs.readFileSync("pages/" + f, "utf8").split("\\n")]);
if (process.argv.slice(2, 5).join(" ") === "logic check --json") {
  const errors = own().flatMap(([path, lines]) => lines.flatMap((l, i) => l.includes("BROKEN") ? [{ path, line: i + 1, message: "compile error: unknown block type BROKEN" }] : []));
  console.log(JSON.stringify(errors.length ? { ok: false, path: errors[0].path, errors } : { ok: true, path: "pages", counts: {} }));
  process.exit(errors.length ? 1 : 0);
}
if (process.argv.slice(2, 5).join(" ") === "logic fmt --check") process.exit(0);
// \`logic lint .\` (#66): dead outputs on each block of the module's own files declared on a line that ends
// in "# dead", and a user whose tag opens no Code Touch when a file says NOTAG; printed as lox prints them.
if (process.argv.slice(2, 5).join(" ") === "logic lint .") {
  // Lint compiles in memory: as slow as a compile (VISLOX_TEST_LOX_DELAY, below).
  for (const end = Date.now() + (+process.env.VISLOX_TEST_LOX_DELAY || 0); Date.now() < end;);
  const dead = own().flatMap(([path, lines]) => lines.flatMap((l) => { const m = l.match(/^(\\w+) = .*# dead$/); return m ? [\`  \${path}: \\\`\${m[1]}\\\` — no wire draws from any of its ports (compiled config, GUI wires included)\`] : []; }));
  const access = own().some(([, lines]) => lines.some((l) => l.includes("NOTAG")))
    ? ["  \\\`Anna\\\` — user with an NFC tag has no access right on any NFC Code Touch, so the tag or code opens nothing."] : [];
  if (dead.length) console.log(\`dead-outputs (\${dead.length})\\n\${dead.join("\\n")}\`);
  if (access.length) console.log(\`no-access-right (1)\\n\${access[0]}\`);
  // no-actuator (#74): a percentage on a circuit of a claimed controller that has no actuator (no NameAI).
  const ed = fs.existsSync("pages/vislox/edits.loxl") ? fs.readFileSync("pages/vislox/edits.loxl", "utf8") : "";
  const base = fs.readFileSync("base.Loxone", "utf8");
  const noAct = [];
  for (const [, slug, set] of ed.matchAll(/^(\\w+)\\.moods = (\\w+)$/gm)) {
    const uuid = ed.match(new RegExp(\`^extern \${slug} = \\\\w+\\\\(uuid: "([^"]+)"\\\\)\`, "m"))?.[1];
    const head = base.split("\\n").find((l) => l.includes(\`U="\${uuid}"\`) && l.includes("<C ")) || "";
    const body = ed.split(\`moods \${set}\\n\`)[1]?.split("\\nend")[0] || "";
    for (const [, name, n, pct] of body.matchAll(/"([^"]*)":[^\\n]*?\\bAQ(\\d+) (\\d+)%/g))
      if (!head.includes(\` NameAI\${n - 1}=\`)) noAct.push(\`  pages/vislox/edits.loxl: \\\`\${slug}\\\` — AQ\${n} has no actuator, so the Miniserver switches it: "\${name}" \${pct}% turns it on, not dimmed. Assign the actuator in Loxone Config's mood editor, or write \\\`on\\\`\`);
  }
  if (noAct.length) console.log(\`no-actuator (\${noAct.length})\\n\${noAct.join("\\n")}\`);
  // tile-broadcast (#78): Flex Küche's tile 5 clicks every linked shade while its click names no target (only while
  // the project has a file TILELINT, so that the other steps' lint stays as it was).
  const outNow = fs.existsSync("out.Loxone") ? fs.readFileSync("out.Loxone", "utf8") : "";
  const bcast = fs.existsSync("TILELINT") && /<btn [^>]*i="5"[^>]*>\\s*<f1 api="SET\\(As;Tg;pulse\\)"\\/>/.test(outNow)
    ? ["  \`Flex Küche\` — tile 5 click: \`SET(As;Tg;pulse)\` names no target, so all 2 API-linked AutoJalousie blocks act on it (AutoJalousie \\"Nord\\", AutoJalousie \\"Süd\\"). List the intended ones with \`to:\`."] : [];
  if (bcast.length) console.log(\`tile-broadcast (1)\\n\${bcast[0]}\`);
  const n = dead.length + access.length + noAct.length + bcast.length;
  console.log(n ? \`lint: \${n} finding\${n === 1 ? "" : "s"}\` : "lint: clean");
  process.exit(n ? 1 : 0);
}
// \`logic test .\` (#67): runs each \`test "…" … end\` of the module's own files; an \`expect\` on a line that says
// FAILTEST reads 0, the others hold. Printed as lox prints it.
if (process.argv.slice(2, 5).join(" ") === "logic test .") {
  for (const end = Date.now() + (+process.env.VISLOX_TEST_LOX_DELAY || 0); Date.now() < end;);
  const out = [];
  let failed = 0, n = 0;
  for (const [, lines] of own()) for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^test "(.*)"/);
    if (!m) continue;
    n++;
    const bad = [];
    let step = 0;
    for (let j = i + 1; j < lines.length && lines[j].trim() !== "end"; j++) {
      const l = lines[j].trim();
      if (/^(tick|step) /.test(l)) step++;
      if (l.startsWith("expect ") && l.includes("FAILTEST")) bad.push(\`    step \${step}: \${l.split("#")[0].trim()} — actual 0\`);
    }
    if (bad.length) failed++;
    out.push(\`test "\${m[1]}" ... \${bad.length ? "FAILED" : "ok"}\`, ...bad);
  }
  if (!n) { console.log("no tests in pages"); process.exit(0); }
  console.log(out.join("\\n") + \`\\n\${n} tests: \${n - failed} passed, \${failed} failed\`);
  process.exit(failed ? 1 : 0);
}
// \`logic adopt <base> --uuid U --as S --module M --lock L\` (#72): as lox, it appends the block's declaration to
// M's file of its page (opened by its page line) and pins its UUID, layout and page in L; the Divide block's
// dividend comes from the virtual input, an extern lox names by its iname. It refuses an input reference.
if (process.argv.slice(2, 4).join(" ") === "logic adopt") {
  const [cfg, uuid, slug, mod, lockPath] = [4, 6, 8, 10, 12].map((i) => process.argv[i]);
  const lines = fs.readFileSync(cfg, "utf8").split("\\n");
  const i = lines.findIndex((l) => l.includes(\`U="\${uuid}"\`) && l.includes("<C "));
  const ty = lines[i].match(/Type="(\\w+)"/)[1], title = lines[i].match(/Title="([^"]*)"/)?.[1] || "";
  if (ty === "InputRef") {
    console.error(\`error: compile error: cannot adopt InputRef "\${title}" (uuid \${uuid}): attribute \\\`Ref\\\` is not understood by the rebuild and would be lost\`);
    process.exit(1);
  }
  if (["Mode", "CalendarEntry", "VirtualIn", "VirtualHttpIn"].includes(ty)) {
    const lk = JSON.parse(fs.readFileSync(lockPath, "utf8"));
    const file = mod + "/objects.loxl";
    let args = "";
    if (ty === "CalendarEntry") {
      const party = Object.entries(lk.objects || {}).find(([,o]) => o.uuid === ${JSON.stringify(PARTY)})?.[0];
      args = lines[i].includes('CalType="-1"') ? ', switches: ' + (party || '"Party"') + ', every: 12-31' : ', switches: holiday, every: 12-24';
    }
    if (ty === "VirtualHttpIn") {
      args = ', Address: "http://192.0.2.30/status", Poll: 60';
      if (Object.values(lk.objects || {}).some((o) => o.uuid === ${JSON.stringify(IMPORT_CMD)})) { console.error("child already managed"); process.exit(1); }
    }
    fs.writeFileSync(file, (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "") + \`\${slug} = \${ty}("\${title}"\${args})\\n\`);
    const ports = {};
    for (let j = i + 1; j < lines.length && !lines[j].includes("</C>"); j++) {
      const co = lines[j].match(/<Co K="([^" ]+)"[^>]* U="([^"]+)"/); if (co) ports[co[1]] = co[2];
    }
    lk.objects = { ...lk.objects, [slug]: { uuid, type: ty, ports, layout: null } };
    if (ty === "VirtualHttpIn") {
      const child = slug + "_command";
      fs.appendFileSync(file, \`\${child} = VirtualHttpInCmd("Imported Command", device: \${slug}, Check: "value")\\n\`);
      lk.objects[child] = { uuid: ${JSON.stringify(IMPORT_CMD)}, type: "VirtualHttpInCmd", ports: {}, layout: null };
    }
    fs.writeFileSync(lockPath, JSON.stringify(lk));
    process.exit(0);
  }
  let p = i;
  while (p >= 0 && !lines[p].includes('Type="Page"')) p--;
  const pageTitle = lines[p].match(/Title="([^"]*)"/)[1], pageUuid = lines[p].match(/U="([^"]+)"/)[1];
  const file = \`\${mod}/\${pageTitle.toLowerCase().replace(/[^a-z0-9]+/g, "_")}.loxl\`;
  const lk = JSON.parse(fs.readFileSync(lockPath, "utf8"));
  const wired = uuid === ${JSON.stringify(DIV)};
  let text = fs.existsSync(file) ? fs.readFileSync(file, "utf8") + "\\n" : \`page "\${pageTitle}"\\n\\n\`;
  if (wired) text += 'extern temperatur = VirtualIn(iname: "VI1")\\n';
  text += \`\${slug} = \${ty}("\${title}"\${wired ? ", In1: temperatur" : ""})\\n\`;
  fs.writeFileSync(file, text);
  const at = lines[i].match(/Px="(-?\\d+)" Py="(-?\\d+)" Px2="(-?\\d+)" Py2="(-?\\d+)"/);
  lk.objects = { ...lk.objects, [slug]: { uuid, type: ty, page_uuid: pageUuid, layout: { px: +at[1], py: +at[2], px2: +at[3], py2: +at[4] } } };
  if (wired) lk.externals = { ...lk.externals, temperatur: { uuid: "10000001-0000-0010-ffff504f94000001", matched_by: "iname" } };
  fs.writeFileSync(lockPath, JSON.stringify(lk));
  console.log(\`adopted uuid \${uuid} as \\\`\${slug}\\\` on page "\${pageTitle}"\`);
  process.exit(0);
}
if (process.argv.slice(2, 4).join(" ") !== "logic compile") { console.error("unexpected: " + process.argv.slice(2).join(" ")); process.exit(2); }
// VISLOX_TEST_LOX_DELAY (ms): compile as slowly as a busy CI runner does.
for (const end = Date.now() + (+process.env.VISLOX_TEST_LOX_DELAY || 0); Date.now() < end;);
let out = fs.readFileSync("base.Loxone", "utf8").split("\\n");
const file = "pages/vislox/edits.loxl";
const src = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "";
const slugs = {};
const KEYS = { In1: "Input1", In2: "Input2", Trigger: "InputTrigger" };
const unq = (v) => v.startsWith('"') ? JSON.parse(v) : v;
// A constant with a unit (#79), in the port's seconds as lox writes it: 30min is 1800.
const scale = (v) => { const m = v.match(/^(-?[\\d.]+)(ms|s|min|h|%|K|°)$/); return m ? String(+m[1] * ({ ms: 0.001, min: 60, h: 3600 }[m[2]] ?? 1)) : v; };
// The block settings (#79) of a declaration's arguments, as the attributes lox writes: \`At: 06:58\` is Sec="25080",
// \`Code: file("x.c")\` the program in pages/vislox/x.c. Only those the declaration binds.
const settingAttrs = (rest) => {
  const a = {};
  const at = rest.match(/\\bAt: (\\d\\d):(\\d\\d)/);
  if (at) a.Sec = String(+at[1] * 3600 + +at[2] * 60);
  const code = rest.match(/\\bCode: file\\("([^"]+)"\\)/);
  if (code) a.Code = fs.readFileSync("pages/vislox/" + code[1], "utf8").replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
  return a;
};
const withAttrs = (line, a) => Object.entries(a).reduce((l, [k, v]) => l.replace(new RegExp(\` \${k}="[^"]*"\`), "").replace(/^(\\s*<C [^>]*?)(\\/?>)/, \`$1 \${k}="\${v}"$2\`), line);
const xml = (v) => v.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
const claimed = [];
// vislox's constants (#81): \`let vx_… = value\`; a claim that names one takes its value.
const lets = Object.fromEntries([...src.matchAll(/^let (\\w+) = (.*)$/gm)].map((m) => [m[1], m[2]]));
for (const line of src.split("\\n")) {
  let m;
  if ((m = line.match(/^extern (\\w+) = \\w+\\(uuid: "([^"]+)"\\)/))) { slugs[m[1]] = m[2]; continue; }
  if (!(m = line.match(/^(\\w+)\\.(\\w+) = (.*)$/))) continue;
  if (m[2] === "schedule") { claimed.push([slugs[m[1]], m[3]]); continue; }
  if (m[2] === "moods" || m[2] === "members" || m[2] === "access" || m[2] === "recipients" || m[2] === "tiles") continue; // below (#74–#78)
  const uuid = slugs[m[1]], i = out.findIndex((l) => l.includes(\`U="\${uuid}"\`));
  if (m[2] === "title") { out[i] = out[i].replace(/Title="[^"]*"/, \`Title="\${xml(unq(m[3]))}"\`); continue; }
  if (m[2] === "room") {
    const place = out.find((l) => l.includes('Type="Place"') && l.includes(\`Title="\${unq(m[3])}"\`)).match(/U="([^"]+)"/)[1];
    const j = out.findIndex((l, n) => n > i && l.includes("<IoData"));
    out[j] = out[j].replace(/Pr="[^"]*"/, \`Pr="\${place}"\`);
    continue;
  }
  const key = KEYS[m[2]] || m[2];
  const j = out.findIndex((l, n) => n > i && l.includes(\`<Co K="\${key}"\`));
  out[j] = out[j].replace(/ Def="[^"]*"/, "").replace(\`<Co K="\${key}"\`, \`<Co K="\${key}" Def="\${scale(unq(lets[m[3]] ?? m[3]))}"\`);
}
// The module's own constants (#81): \`let x = v\` of its files; an extern's \`x.Port = name\` there takes its value.
const modLets = {};
for (const [, lines] of own()) for (const l of lines) { const m = l.match(/^let (\\w+)\\s*=\\s*([^#\\s]+)/); if (m) modLets[m[1]] = m[2]; }
for (const [, lines] of own()) {
  const ext = {};
  for (const l of lines) {
    let m;
    if ((m = l.match(/^extern (\\w+) = \\w+\\(uuid: "([^"]+)"\\)/))) { ext[m[1]] = m[2]; continue; }
    if (!(m = l.match(/^(\\w+)\\.(\\w+) = (\\w+)\\s*$/)) || !ext[m[1]] || modLets[m[3]] == null) continue;
    const key = KEYS[m[2]] || m[2], i = out.findIndex((x) => x.includes(\`U="\${ext[m[1]]}"\`));
    const j = out.findIndex((x, n) => n > i && x.includes(\`<Co K="\${key}"\`));
    out[j] = out[j].replace(/ Def="[^"]*"/, "").replace(\`<Co K="\${key}"\`, \`<Co K="\${key}" Def="\${scale(modLets[m[3]])}"\`);
  }
}
// Switching times (#73): each \`schedule\` set of vislox's file, written onto the timers its claims name as lox
// writes them: a column per mode the set names and the seven weekdays, in \`Mode=\` order, an <Entry> per interval.
const sets = {};
{
  let cur = null;
  for (const line of src.split("\\n")) {
    const m = line.match(/^schedule (\\w+)$/);
    if (m) cur = sets[m[1]] = [];
    else if (line === "end") cur = null;
    else if (cur && line.trim()) cur.push(line.trim());
  }
}
const DAYW = { holiday: [1], vacation: [2], dayoff: [3], mon: [4], tue: [5], wed: [6], thu: [7], fri: [8], sat: [9], sun: [10], weekdays: [4, 5, 6, 7, 8], weekend: [9, 10], daily: [4, 5, 6, 7, 8, 9, 10] };
const modeU = (n) => \`00000000-0000-\${n.toString(16).padStart(4, "0")}-1500000000000000\`;
for (const [uuid, name] of claimed) {
  const cols = new Map([4, 5, 6, 7, 8, 9, 10].map((n) => [modeU(n), { n: n - 1, ivs: [] }]));
  for (const l of sets[name]) {
    const [, days, rest] = l.match(/^(.*?) (off|\\d\\d:\\d\\d-.*)$/);
    const ivs = rest === "off" ? [] : rest.split(", ").map((x) => {
      const m = x.match(/^(\\d\\d):(\\d\\d)-(\\d\\d):(\\d\\d)(?: = (-?[\\d.]+))?( activate)?$/);
      return { from: +m[1] * 60 + +m[2], to: +m[3] * 60 + +m[4], v: m[5] ?? "1", act: !!m[6] };
    });
    for (const d of days.match(/"[^"]*"|\\w+/g)) {
      const us = d.startsWith('"')
        ? [out.find((x) => x.includes('Type="Mode"') && x.includes(\`Title="\${JSON.parse(d)}"\`))].map((x) => [x.match(/U="([^"]+)"/)[1], +(x.match(/ Mode="(-?\\d+)"/)?.[1] ?? 0)])
        : DAYW[d].map((n) => [modeU(n), n - 1]);
      for (const [u, n] of us) { if (!cols.has(u)) cols.set(u, { n, ivs: [] }); cols.get(u).ivs.push(...ivs); }
    }
  }
  const order = [...cols].sort((a, b) => a[1].n - b[1].n);
  const entries = order.flatMap(([, c], ix) => c.ivs.sort((a, b) => a.from - b.from).map((iv) =>
    \`\\t\\t\\t\\t<Entry\${iv.from ? \` From="\${iv.from}"\` : ""} To="\${iv.to}"\${ix ? \` Ix="\${ix}"\` : ""}\${iv.act ? ' NeedActivate="true"' : ""} V="\${iv.v}"/>\`));
  const i = out.findIndex((l) => l.includes(\`U="\${uuid}"\`) && l.includes("<C "));
  const j = out.findIndex((l, n) => n > i && l.trim() === "</C>");
  const kept = out.slice(i + 1, j).filter((l) => !l.includes("<Entry"));
  const head = out[i].replace(/ Modes="[^"]*"/, "").replace(/ N="\\d+"/, "").replace(/>$/, \` N="\${entries.length}" Modes="\${order.map(([u]) => u).join(",")}">\`);
  out.splice(i, j - i, head, ...kept, ...entries);
}
// Lighting moods (#74): each \`moods\` set of vislox's file, written onto the controllers its claims name as lox
// writes them: a mood the controller has keeps its scene id, a new one takes the lowest free one, a mood left out
// goes, and the off mood stays where it is. Each circuit's value follows its kind (the Q's top four bits).
{
  const msets = {};
  let cur = null;
  for (const line of src.split("\\n")) {
    const m = line.match(/^moods (\\w+)$/);
    if (m) cur = msets[m[1]] = [];
    else if (line === "end") cur = null;
    else if (cur && line.trim()) { const x = line.trim().match(/^"((?:[^"\\\\]|\\\\.)*)": (.*)$/); cur.push([JSON.parse('"' + x[1] + '"'), x[2]]); }
  }
  const P = 2 ** 28;
  const enc = (prefix, v) => {
    let m;
    if (v === "off") return prefix * P;
    if (v === "on") return prefix === 0 ? 1 : null;
    if ((m = v.match(/^(\\d+)%$/))) return prefix <= 1 ? prefix * P + +m[1] : null;
    if ((m = v.match(/^rgb\\((\\d+), (\\d+), (\\d+)\\)$/))) return prefix >= 4 ? prefix * P + +m[1] + 1000 * +m[2] + 1000000 * +m[3] : null;
    if ((m = v.match(/^temp\\((\\d+)%, (\\d+)K\\)$/))) return prefix >= 5 ? prefix * P + 200000000 + 10000 * +m[1] + +m[2] : null;
    return null;
  };
  for (const [, slug, set] of src.matchAll(/^(\\w+)\\.moods = (\\w+)$/gm)) {
    const uuid = slugs[slug];
    const i = out.findIndex((l) => l.includes(\`U="\${uuid}"\`) && l.includes("<C "));
    const a = out.findIndex((l, n) => n > i && l.includes("<LightscenesC")), b = out.findIndex((l, n) => n > a && l.includes("</LightscenesC>"));
    const old = out.slice(a + 1, b);
    const attr = (l, k) => l.match(new RegExp(\` \${k}="([^"]*)"\`))?.[1];
    const prefixes = Array.from({ length: 18 }, (_, n) => Math.floor(+attr(old[0], \`Q\${n + 1}\`) / P));
    const offAt = old.findIndex((l) => attr(l, "SID") === "778");
    const keep = (l) => msets[set].some(([nm]) => nm === attr(l, "Name"));
    const sids = new Set([776, 777, 778, ...old.filter(keep).map((l) => +attr(l, "SID"))]);
    const moods = msets[set].map(([name, settings]) => {
      const parts = settings.match(/(all|AQ\\d+) (rgb\\([^)]*\\)|temp\\([^)]*\\)|[^,]+)/g);
      const all = parts.find((p) => p.startsWith("all "))?.slice(4) ?? "off";
      const qs = prefixes.map((pf, n) => { const own = parts.find((p) => p.startsWith(\`AQ\${n + 1} \`)); return own ? enc(pf, own.slice(own.indexOf(" ") + 1)) : enc(pf, all) ?? pf * P; });
      let l = old.find((x) => attr(x, "Name") === name && attr(x, "SID") !== "778");
      if (!l) {
        let sid = 1;
        while (sids.has(sid)) sid++;
        sids.add(sid);
        l = \`\\t\\t\\t\\t\\t<LightsceneC Name="\${xml(name)}" UUID="cccccc\${String(sid).padStart(2, "0")}-0000-0000-ffff504f94000001" Outputs="18" SID="\${sid}" CID="\${30 + sid}" \${qs.map((_, n) => \`Q\${n + 1}="0"\`).join(" ")}/>\`;
      }
      return qs.reduce((x, q, n) => x.replace(new RegExp(\` Q\${n + 1}="[^"]*"\`), \` Q\${n + 1}="\${q}"\`), l);
    });
    if (offAt >= 0) moods.splice(offAt === old.length - 1 ? moods.length : Math.min(offAt, moods.length), 0, old[offAt]);
    out.splice(a + 1, b - a - 1, ...moods);
    out[a] = out[a].replace(/Num="\\d+"/, \`Num="\${moods.length}"\`);
  }
}
// vislox's new blocks: one file per page (page "…"), a declaration per block. Each gets a UUID in the
// lock (kept by slug across compiles) and is drawn on its page with the constants it is given.
// A scratch compile (--lock, --out elsewhere) leaves the project's lock and the UUIDs minted alone.
const lockAt = process.argv.indexOf("--lock");
const lockFile = lockAt > 0 ? process.argv[lockAt + 1] : "logic.lock.json";
const lock = fs.existsSync(lockFile) ? JSON.parse(fs.readFileSync(lockFile, "utf8")) : { objects: {} };
const objects = {};
// vislox's lifecycle lines (#80): \`moved a -> b\` moves a's lock entry to b while the lock has a and not b (the
// block keeps its UUIDs); \`removed x\` of a block the lock manages and the base has deletes it, with a tombstone.
const mine = (fs.existsSync("pages/vislox") ? fs.readdirSync("pages/vislox") : []).filter((f) => f.endsWith(".loxl"))
  .flatMap((f) => fs.readFileSync("pages/vislox/" + f, "utf8").split("\\n"));
for (const line of mine) {
  const m = line.match(/^moved (vx_\\w+) -> (vx_\\w+)$/);
  if (m && lock.objects[m[1]] && !lock.objects[m[2]]) { lock.objects[m[2]] = lock.objects[m[1]]; delete lock.objects[m[1]]; }
}
// Like lox, a block added again gets a UUID it never had: the count of UUIDs minted is kept.
const mintFile = ${JSON.stringify(join(loxDir, "minted"))};
let n = fs.existsSync(mintFile) ? +fs.readFileSync(mintFile, "utf8") : 0;
// An expression's top block (#82), as lox makes it, drawn on the page at out[pageAt]: one block of the type of the
// expression's top operator, labelled with it (\`<=\` as ≤, \`<\` as ＜), its UUID kept by slug; an expression's is
// expression-owned in the lock. Returns its output connector's UUID.
const exprBlock = (slug, text, pageAt, owned, title) => {
  const [ty, q] = / or /.test(text) ? ["Or", "Q"] : / and /.test(text) ? ["And", "Q"] : /^not /.test(text) ? ["Not", "Q"]
    : />=/.test(text) ? ["GreaterEqual", "Q"] : /<=/.test(text) ? ["LessEqual", "Q"] : />/.test(text) ? ["Greater", "Q"]
    : /</.test(text) ? ["Less", "Q"] : /==/.test(text) ? ["Equal", "Q"] : /!=/.test(text) ? ["NotEqual", "Q"] : ["Formula", "AQ"];
  const uuid = lock.objects[slug]?.uuid || \`eeeeee\${String(++n).padStart(2, "0")}-0000-0000-ffff504f94000001\`;
  const page_uuid = out[pageAt].match(/U="([^"]+)"/)[1];
  const layout = lock.objects[slug]?.layout || { px: 9600, py: 960, px2: 10944, py2: 1656 };
  objects[slug] = { uuid, type: ty, page_uuid, layout, ...(owned ? { expr_owned: true } : {}) };
  const label = title ?? text.replace(/<=/g, "≤").replace(/</g, "＜").replace(/%/g, "％");
  const co = (k, i) => \`\\t\\t\\t\\t<Co K="\${k}" U="\${uuid.slice(0, 9)}0000-00\${i}0-ffff504f94000001"/>\`;
  out.splice(pageAt + 1, 0, \`\\t\\t\\t<C Type="\${ty}" V="178" U="\${uuid}" Title="\${xml(label)}" Px="96" Py="96" Px2="1440" Py2="792" Nio="3">\`, co("I1", 1), co("I2", 2), co(q, 3), "\\t\\t\\t</C>");
  return \`\${uuid.slice(0, 9)}0000-0030-ffff504f94000001\`;
};
// Templates (#83): \`template Name(params)\` … \`end\` of the module's files and vislox's templates.loxl, each body
// block \`name = Type("title", …)\` (a title may interpolate a string parameter, \`{label}\`).
const templates = {};
for (const path of [...own().map(([p]) => p), "pages/vislox/templates.loxl"]) {
  if (!fs.existsSync(path)) continue;
  let cur = null;
  for (const line of fs.readFileSync(path, "utf8").split("\\n")) {
    const h = line.match(/^template (\\w+)\\((.*?)\\)(?: ->.*)?$/);
    if (h) { cur = templates[h[1]] = { params: {}, body: [] }; for (const [, k, v] of h[2].matchAll(/(\\w+): \\w+(?: = ("(?:[^"\\\\]|\\\\.)*"|[^,]+))?/g)) cur.params[k] = v; continue; }
    if (line.trim() === "end") { cur = null; continue; }
    const b = cur && line.match(/^\\s+(\\w+) = (\\w+)\\((?:"((?:[^"\\\\]|\\\\.)*)")?/);
    if (b) cur.body.push({ name: b[1], ty: b[2], title: b[3] });
  }
}
// An instance of vislox's: each body block as \`<instance>_<name>\`, its UUID kept by slug, on the call's page.
const instance = (slug, tname, rest, pageAt) => {
  const t = templates[tname];
  if (!t) { console.error(\`error: compile error: \\\`\${slug} = \${tname}(…)\\\` instantiates unknown template \\\`\${tname}\\\`\`); process.exit(1); }
  const args = { ...t.params };
  const lab = rest.match(/^"((?:[^"\\\\]|\\\\.)*)"/);
  if (lab) args.label = \`"\${lab[1]}"\`;
  for (const [, k, v] of rest.matchAll(/(\\w+): ("(?:[^"\\\\]|\\\\.)*"|[^,]+)/g)) args[k] = v.trim();
  for (const b of t.body) {
    const s = \`\${slug}_\${b.name}\`;
    const uuid = lock.objects[s]?.uuid || \`cccc\${String(++n).padStart(4, "0")}-0000-0000-ffff504f94000001\`;
    const page_uuid = out[pageAt].match(/U="([^"]+)"/)[1];
    objects[s] = { uuid, type: b.ty, page_uuid, layout: lock.objects[s]?.layout || { px: 2880, py: 2880, px2: 4224, py2: 3576 } };
    const title = (b.title ?? s).replace(/\\{(\\w+)\\}/g, (_, k) => (args[k] ? JSON.parse(args[k]) : ""));
    out.splice(pageAt + 1, 0, \`\\t\\t\\t<C Type="\${b.ty}" V="178" U="\${uuid}" Title="\${xml(title)}" Px="96" Py="96" Px2="1440" Py2="792" Nio="2"><Co K="Q" U="\${uuid.slice(0, 9)}0000-0030-ffff504f94000001"/></C>\`);
  }
};
// Objects of the object tree (#84): vislox's _periphery.loxl, with no page. A device goes into its folder, an I/O
// below the device its \`device:\` names (an extern of edits.loxl, or a device declared above it), each with the
// settings its declaration binds as attributes.
// Operating modes and calendar entries (#86): a mode is numbered one below the lowest in use, in file order; an
// entry's mode (a day word, a slug, a title) is its CalType=, its date the Mode=/Month/Day/… attributes it stores.
const modeOf = (l) => +((l.match(/ Mode="(-?\\d+)"/) || [0, 0])[1]);
let nextMode = Math.min(0, ...out.filter((l) => l.includes('Type="Mode"')).map(modeOf));
const modeNo = {};
const DAYS = { holiday: 0, vacation: 1, dayoff: 2, mon: 3, tue: 4, wed: 5, thu: 6, fri: 7, sat: 8, sun: 9, heating: 10, cooling: 11 };
const since2009 = (s) => { const [y, m, d] = s.split("-").map(Number); return Math.round((Date.UTC(y, m - 1, d) - Date.UTC(2009, 0, 1)) / 864e5); };
const entryAttrs = (rest) => {
  const sw = rest.match(/switches: ("(?:[^"\\\\]|\\\\.)*"|\\w+)/);
  let no = 0;
  if (sw && sw[1].startsWith('"')) { const l = out.find((x) => x.includes('Type="Mode"') && x.includes(\`Title="\${xml(JSON.parse(sw[1]))}"\`)); no = l ? modeOf(l) : 0; }
  else if (sw) no = sw[1] in DAYS ? DAYS[sw[1]] : modeNo[sw[1]] ?? 0;
  let a = no ? \` CalType="\${no}"\` : "", m;
  if ((m = rest.match(/every: (\\d\\d)-(\\d\\d)/))) a += \` Month="\${+m[1]}" Day="\${+m[2]}"\`;
  else if ((m = rest.match(/easter: (-?\\d+)/))) a += \` Mode="1" EasterOffset="\${m[1]}"\`;
  else if ((m = rest.match(/from: (\\d{4}-\\d\\d-\\d\\d), to: (\\d{4}-\\d\\d-\\d\\d)/))) a += \` Mode="3" From="\${since2009(m[1])}" To="\${since2009(m[2])}"\`;
  else if ((m = rest.match(/from: (\\d\\d)-(\\d\\d), to: (\\d\\d)-(\\d\\d)/))) a += \` Mode="4" Month="\${+m[1]}" Day="\${+m[2]}" Month2="\${+m[3]}" Day2="\${+m[4]}"\`;
  return a + ' WeekDayInMonth="1"';
};
const FOLDER = { VirtualIn: "VirtualInCaption", VirtualHttpIn: "VirtualInCaption", VirtualUdpIn: "VirtualInCaption", VirtualTextIn: "VirtualInCaption", VirtualOut: "VirtualOutCaption", ModbusServer: "WeatherCaption", Mode: "ModeCaption", CalendarEntry: "CalendarCaption" };
if (fs.existsSync("pages/vislox/_periphery.loxl")) {
  for (const line of fs.readFileSync("pages/vislox/_periphery.loxl", "utf8").split("\\n")) {
    const m = line.match(/^(vx_\\w+) = (\\w+)\\((.*)\\)$/);
    if (!m) continue;
    const [, slug, ty, rest] = m;
    const uuid = lock.objects[slug]?.uuid || \`dddddd\${String(++n).padStart(2, "0")}-0000-0000-ffff504f94000001\`;
    objects[slug] = { uuid, type: ty, ports: {}, layout: null };
    const dev = rest.match(/device: (\\w+)/), at = dev ? (slugs[dev[1]] || objects[dev[1]]?.uuid) : null;
    const i = out.findIndex((l) => l.includes("<C ") && (at ? l.includes(\`U="\${at}"\`) : l.includes(\`Type="\${FOLDER[ty]}"\`)));
    if (i < 0) { console.error(\`error: compile error: \\\`\${slug}\\\`: no \${at ? "device " + at : FOLDER[ty]} in the base\`); process.exit(1); }
    const title = rest.match(/^"((?:[^"\\\\]|\\\\.)*)"/);
    const attrs = ty === "Mode" ? \` Mode="\${(modeNo[slug] = --nextMode)}"\` : ty === "CalendarEntry" ? entryAttrs(rest) : [...rest.matchAll(/(\\w+): ("(?:[^"\\\\]|\\\\.)*"|[^,]+)/g)].filter(([, k]) => k !== "device")
      .map(([, k, v]) => \` \${k}="\${xml(v.startsWith('"') ? JSON.parse(v) : v === "on" ? "true" : v === "off" ? "false" : scale(v))}"\`).join("");
    const kept = out.findIndex((l) => l.includes("<C ") && l.includes(\`U="\${uuid}"\`));
    if (kept >= 0) {
      objects[slug] = { ...lock.objects[slug] };
      if (ty === "Mode") modeNo[slug] = modeOf(out[kept]);
      if (title) out[kept] = out[kept].replace(/Title="[^"]*"/, \`Title="\${xml(JSON.parse('"' + title[1] + '"'))}"\`);
      if (ty !== "Mode") out[kept] = withAttrs(out[kept], ty === "CalendarEntry" ? Object.fromEntries([...entryAttrs(rest).matchAll(/ (\\w+)="([^"]*)"/g)].map(([, k, v]) => [k, v])) : settingAttrs(rest));
      continue;
    }
    out.splice(i + 1, 0, \`\\t\\t\\t<C Type="\${ty}" V="178" U="\${uuid}" Title="\${xml(title ? JSON.parse('"' + title[1] + '"') : slug)}"\${attrs}>\`, "\\t\\t\\t</C>");
  }
}
// New pages (#88): a page line with \`room:\` creates the page when no page has its title, in the room it names (by
// title, as lox resolves it; none or two fail the compile), its UUID kept by title in the lock's \`pages\`.
const pages = {};
for (const f of fs.existsSync("pages/vislox") ? fs.readdirSync("pages/vislox") : []) {
  if (!f.endsWith(".loxl")) continue;
  const m = fs.readFileSync("pages/vislox/" + f, "utf8").match(/^page "((?:[^"\\\\]|\\\\.)*)" \\(room: "((?:[^"\\\\]|\\\\.)*)"(?:, category: "(?:[^"\\\\]|\\\\.)*")?\\)$/m);
  if (!m) continue;
  const [title, room] = [JSON.parse('"' + m[1] + '"'), JSON.parse('"' + m[2] + '"')];
  const places = out.filter((l) => l.includes('Type="Place"') && l.includes(\`Title="\${xml(room)}"\`));
  if (places.length !== 1) { console.error(\`error: compile error: \${places.length ? places.length + " rooms are" : "no room"} titled "\${room}"\`); process.exit(1); }
  if (out.some((l) => l.includes('Type="Page"') && l.includes(\`Title="\${xml(title)}"\`))) continue;
  pages[title] = (lock.pages || {})[title] || \`ffffff\${String(++n).padStart(2, "0")}-0000-0000-ffff504f94000001\`;
  const at = out.findIndex((l) => l.includes('<C Type="Page"'));
  out.splice(at, 0, \`\\t\\t<C Type="Page" V="178" U="\${pages[title]}" Title="\${xml(title)}" WF="16384">\`, \`\\t\\t\\t<IoData Pr="\${places[0].match(/U="([^"]+)"/)[1]}"/>\`, "\\t\\t</C>");
}
for (const f of fs.existsSync("pages/vislox") ? fs.readdirSync("pages/vislox") : []) {
  if (f === "edits.loxl" || f === "templates.loxl" || f === "_periphery.loxl" || f === "_hardware.loxl" || !f.endsWith(".loxl")) continue;
  let pageAt = -1, instancing = false;
  for (const line of fs.readFileSync("pages/vislox/" + f, "utf8").split("\\n")) {
    let m;
    if ((m = line.match(/^page "(.*?)"(?: \\(.*\\))?$/))) { pageAt = out.findIndex((l) => l.includes('Type="Page"') && l.includes(\`Title="\${m[1]}"\`)); continue; }
    if (line.startsWith("#")) { instancing = line.startsWith("# Template instances"); continue; }
    if (instancing && (m = line.match(/^(vx_\\w+) = (\\w+)\\((.*)\\)$/))) { instance(m[1], m[2], m[3], pageAt); continue; }
    // A named signal (#82): its top block, labelled with its .title or its expression.
    if ((m = line.match(/^(vx_\\w+) = (.+)$/)) && !/^\\w+\\(/.test(m[2])) {
      const t = fs.readFileSync("pages/vislox/" + f, "utf8").match(new RegExp(\`^\${m[1]}\\\\.title = "((?:[^"\\\\\\\\]|\\\\\\\\.)*)"$\`, "m"));
      exprBlock(m[1], m[2], pageAt, false, t ? JSON.parse('"' + t[1] + '"') : null);
      continue;
    }
    if (!(m = line.match(/^(vx_\\w+) = (\\w+)\\((.*)\\)$/))) continue;
    const [, slug, ty, rest] = m;
    // A block adopted here (#72) is drawn in the base already: it keeps it, with the name its declaration gives.
    const kept = lock.objects[slug]?.uuid, k = kept ? out.findIndex((l) => l.includes(\`U="\${kept}"\`) && l.includes("<C ")) : -1;
    if (k >= 0) {
      objects[slug] = { ...lock.objects[slug] };
      const t = rest.match(/^"((?:[^"\\\\]|\\\\.)*)"/);
      if (t) out[k] = out[k].replace(/Title="[^"]*"/, \`Title="\${xml(JSON.parse('"' + t[1] + '"'))}"\`);
      out[k] = withAttrs(out[k], settingAttrs(rest));
      continue;
    }
    const uuid = lock.objects[slug]?.uuid || \`bbbbbb\${String(++n).padStart(2, "0")}-0000-0000-ffff504f94000001\`;
    const page_uuid = out[pageAt].match(/U="([^"]+)"/)[1];
    objects[slug] = { uuid, type: ty, page_uuid, layout: lock.objects[slug]?.layout || { px: 96, py: 96, px2: 1440, py2: 792 } };
    // A push notification's block (#77): it mirrors the notification (its title, Ref=) and carries the message.
    if (ty === "OutputRefLM") {
      const notif = slugs[rest.match(/mirrors: (\\w+)/)[1]];
      const ntitle = out.find((l) => l.includes(\`U="\${notif}"\`)).match(/Title="([^"]*)"/)[1];
      const txt = (k) => { const x = rest.match(new RegExp(\`\${k}: "((?:[^"\\\\\\\\]|\\\\\\\\.)*)"\`)); return x ? \` \${k}="\${xml(JSON.parse('"' + x[1] + '"'))}"\` : ""; };
      const mt = rest.match(/MinimumTime: (\\d+)(ms|s|min|h)?/);
      const ms = mt ? +mt[1] * { ms: 1, s: 1000, min: 60000, h: 3600000 }[mt[2] || "s"] : 5000;
      out.splice(pageAt + 1, 0, \`\\t\\t\\t<C Type="OutputRefLM" V="178" U="\${uuid}" Title="\${ntitle}" Px="96" Py="96" Px2="1440" Py2="792" Cl="0,0,0" Nio="2" Ref="\${notif}"><Co K="AI" U="\${uuid.slice(0, 9)}0000-0010-ffff504f94000001"/><Co K="AQ" U="\${uuid.slice(0, 9)}0000-0020-ffff504f94000001"/><LoggerMailer RefNotification="\${notif}"\${txt("On")}\${txt("Off")} MinimumTime="\${ms}" NL="1"/></C>\`);
      continue;
    }
    const title = rest.match(/^"((?:[^"\\\\]|\\\\.)*)"/);
    const cos = [...rest.matchAll(/(\\w+): (-?[\\d.]+(?:ms|s|min|h|%|K|°)?)(?=[,)]|$)/g)].map(([, k, v], i) => \`<Co K="\${KEYS[k] || k}" Def="\${scale(v)}" U="\${uuid.slice(0, 9)}0000-00\${i}0-ffff504f94000001"/>\`);
    out.splice(pageAt + 1, 0, withAttrs(\`\\t\\t\\t<C Type="\${ty}" V="178" U="\${uuid}" Title="\${xml(title ? JSON.parse('"' + title[1] + '"') : slug)}" Px="96" Py="96" Px2="1440" Py2="792" Nio="2">\${cos.join("")}</C>\`, settingAttrs(rest)));
  }
}
// vislox's expressions onto inputs (#82): \`x.Port <- <expression>\` in edits.loxl, a block named after the sink's
// connector key, on the sink's page, wired into it.
for (const line of src.split("\\n")) {
  const m = line.match(/^(\\w+)\\.(\\w+) <- (.+)$/);
  if (!m || /^\\w+(\\.\\w+)?$/.test(m[3])) continue;
  const sink = slugs[m[1]] || objects[m[1]]?.uuid;
  let i = out.findIndex((l) => l.includes(\`U="\${sink}"\`) && l.includes("<C "));
  let pageAt = i;
  while (pageAt >= 0 && !out[pageAt].includes('Type="Page"')) pageAt--;
  const key = out[out.findIndex((l, x) => x > i && l.includes(\`<Co K="\${m[2].replace(/^In(\\d)$/, "I$1")}"\`))].match(/K="([^"]+)"/)[1];
  const top = / or /.test(m[3]) ? "or" : / and /.test(m[3]) ? "and" : /^not /.test(m[3]) ? "not" : />=/.test(m[3]) ? "ge" : /<=/.test(m[3]) ? "le" : />/.test(m[3]) ? "gt" : /</.test(m[3]) ? "lt" : /==/.test(m[3]) ? "eq" : /!=/.test(m[3]) ? "ne" : "f";
  const q = exprBlock(\`\${m[1]}_\${key.toLowerCase()}__\${top}1\`, m[3], pageAt, true, null);
  i = out.findIndex((l) => l.includes(\`U="\${sink}"\`) && l.includes("<C "));
  const to = out.findIndex((l, x) => x > i && l.includes(\`<Co K="\${key}"\`));
  out[to] = out[to].replace(/\\/>$/, \`><In Input="\${q}"/></Co>\`);
}
// The blocks the rest of the module manages stay in the lock, with the titles their files give them.
for (const [s, o] of Object.entries(lock.objects)) if (!s.startsWith("vx_")) objects[s] = o;
const tombs = { ...(lock.removed || {}) };
for (const line of mine) {
  const m = line.match(/^removed (vx_\\w+)$/), o = m && !objects[m[1]] && lock.objects[m[1]];
  if (o && out.some((l) => l.includes(\`U="\${o.uuid}"\`))) tombs[o.uuid] = { slug: m[1], type: o.type };
}
for (const [u, t] of Object.entries(tombs)) {
  if (!t.slug.startsWith("vx_")) continue;
  const i = out.findIndex((l) => l.includes(\`U="\${u}"\`) && l.includes("<C "));
  if (i < 0 || objects[t.slug]?.uuid === u) { delete tombs[u]; continue; }
  const one = /\\/>\\s*$/.test(out[i]) || out[i].includes("</C>");
  out.splice(i, one ? 1 : out.findIndex((l, n) => n > i && l.trim() === "</C>") - i + 1);
}
// The module's own files declare \`flur\`. When they no longer do, the compile is refused while the lock manages
// it (#70), unless removals are allowed: then the lock forgets it, with a tombstone, and the config loses it.
let removed = Object.keys(tombs).length ? tombs : undefined;
if (objects.flur && !own().some(([, lines]) => lines.some((l) => /^flur = /.test(l)))) {
  if (!process.argv.includes("--allow-removals")) {
    console.error("error: compile error: managed block \`flur\` is in the lockfile but not in the source. To delete it, add \`removed flur\` (or pass --allow-removals); to rename it, add \`moved flur -> <new_slug>\`.");
    process.exit(1);
  }
  removed = { ...removed, [objects.flur.uuid]: { slug: "flur", type: "And" } };
  const i = out.findIndex((l) => l.includes(\`U="\${objects.flur.uuid}"\`));
  out.splice(i, out.findIndex((l, n) => n > i && l.trim() === "</C>") - i + 1);
  delete objects.flur;
}
for (const [, lines] of own()) for (const line of lines) {
  const m = line.match(/^(\\w+) = \\w+\\("(.*)"\\)$/), o = m && objects[m[1]];
  const i = o ? out.findIndex((l) => l.includes(\`U="\${o.uuid}"\`)) : -1;
  if (i >= 0) out[i] = out[i].replace(/Title="[^"]*"/, \`Title="\${xml(m[2])}"\`);
}
// Defaults (#81): \`defaults T(P: v, …)\` of any file gives each managed block of type T whose declaration does not
// bind P the value (a constant's, by its name).
{
  const all = [...own().flatMap(([, l]) => l), ...mine];
  for (const l of all) {
    const d = l.match(/^defaults (\\w+)\\((.*)\\)\\s*(#.*)?$/);
    if (!d) continue;
    for (const [, port, v] of d[2].matchAll(/(\\w+): ([^,]+)/g)) {
      for (const [slug, o] of Object.entries({ ...lock.objects, ...objects })) {
        if (o.type !== d[1] || all.some((x) => x.startsWith(\`\${slug} = \`) && x.includes(\`\${port}:\`))) continue;
        const key = KEYS[port] || port, i = out.findIndex((x) => x.includes(\`U="\${o.uuid}"\`) && x.includes("<C "));
        const j = i < 0 ? -1 : out.findIndex((x, n) => n > i && x.includes(\`<Co K="\${key}"\`));
        if (j < 0) continue;
        out[j] = out[j].replace(/ Def="[^"]*"/, "").replace(\`<Co K="\${key}"\`, \`<Co K="\${key}" Def="\${scale(lets[v.trim()] ?? modLets[v.trim()] ?? v.trim())}"\`);
      }
    }
  }
}
const relayout = process.argv.flatMap((a, i) => process.argv[i - 1] === "--relayout" ? [a] : []);
const cols = {};
for (const o of Object.values(objects)) {
  const pg = out.find((l) => l.includes('Type="Page"') && l.includes(\`U="\${o.page_uuid}"\`));
  if (!pg || !relayout.some((t) => pg.includes(\`Title="\${t}"\`))) continue;
  const w = o.layout.px2 - o.layout.px, h = o.layout.py2 - o.layout.py;
  // Ten a row, from (9600, 960).
  const c = (cols[o.page_uuid] = (cols[o.page_uuid] ?? -1) + 1);
  const x = 9600 + 1920 * (c % 10), y = 960 + 960 * Math.floor(c / 10);
  o.superseded_layout = o.layout;
  o.layout = { px: x, py: y, px2: x + w, py2: y + h };
}
for (const o of Object.values(objects)) {
  const i = out.findIndex((l) => l.includes(\`U="\${o.uuid}"\`));
  const L = o.layout;
  if (i >= 0 && L) out[i] = out[i].replace(/Px="-?\\d+" Py="-?\\d+" Px2="-?\\d+" Py2="-?\\d+"/, \`Px="\${L.px}" Py="\${L.py}" Px2="\${L.px2}" Py2="\${L.py2}"\`);
}
// Members of central blocks (#75): \`x.members = (a, b)\` makes rec= the members' UUIDs in that order (right before
// SpStates=, as lox writes it); \`()\` takes rec= out.
for (const [, slug, list] of src.matchAll(/^(\\w+)\\.members = \\(([^)]*)\\)$/gm)) {
  const uuidOf = (s) => slugs[s] || objects[s]?.uuid;
  const rec = list.split(",").map((x) => x.trim()).filter(Boolean).map(uuidOf).join(",");
  const i = out.findIndex((l) => l.includes(\`U="\${uuidOf(slug)}"\`) && l.includes("<C "));
  out[i] = out[i].replace(/ rec="[^"]*"/, "");
  if (rec) out[i] = out[i].replace(/ SpStates=/, \` rec="\${rec}" SpStates=\`);
}
// Access rights (#76): \`x.access = set\` makes the set the Code Touch's rights, as lox writes them: every user's and
// group's TIM= on it goes (a <PRM> with ACT= keeps that), and each principal the set names by title gets TIM= of the
// schedule it names by title. A title that names nothing or two objects fails the compile, as with lox.
{
  const asets = {};
  let cur = null;
  for (const line of src.split("\\n")) {
    const m = line.match(/^access (\\w+)$/);
    if (m) cur = asets[m[1]] = [];
    else if (line === "end") cur = null;
    else if (cur && line.trim()) { const x = line.trim().match(/^(user|group) "([^"]*)": "([^"]*)"$/); cur.push([x[1], x[2], x[3]]); }
  }
  const one = (type, title) => {
    const hits = out.filter((l) => l.includes(\`<C Type="\${type}"\`) && l.includes(\` Title="\${xml(title)}"\`));
    if (hits.length !== 1) { console.error(\`error: compile error: \${hits.length} \${type} titled "\${title}"\`); process.exit(1); }
    return hits[0].match(/ U="([^"]+)"/)[1];
  };
  for (const [, slug, set] of src.matchAll(/^(\\w+)\\.access = (\\w+)$/gm)) {
    const door = slugs[slug];
    const want = new Map(asets[set].map(([k, p, sch]) => [one(k === "user" ? "User" : "UserGroup", p), one("SwitchingTimer", sch)]));
    out = out.flatMap((l) => {
      if (!l.includes(\`<PRM REF="\${door}"\`)) return [l];
      const kept = l.replace(/ TIM="[^"]*"/, "");
      return / ACT="/.test(kept) ? [kept] : [];
    });
    for (const [who, sch] of want) {
      const i = out.findIndex((l) => l.includes(\` U="\${who}"\`) && l.includes("<C "));
      const j = out.findIndex((l, n) => n > i && l.includes("</PH>"));
      const k = out.findIndex((l, n) => n > i && n < j && l.includes(\`<PRM REF="\${door}"\`));
      if (k >= 0) out[k] = out[k].replace(\`<PRM REF="\${door}"\`, \`<PRM REF="\${door}" TIM="\${sch}"\`);
      else out.splice(j, 0, \`\\t\\t\\t\\t\\t<PRM REF="\${door}" TIM="\${sch}"/>\`);
    }
  }
}
// Recipients (#77): \`x.recipients = set\` makes the set the notification's recipients, as lox writes them: every
// ACT= on it goes, and each principal the set names by title gets <PRM REF="…" ACT="3"/>. Full access is never listed.
{
  const rsets = {};
  let cur = null;
  for (const line of src.split("\\n")) {
    const m = line.match(/^recipients (\\w+)$/);
    if (m) cur = rsets[m[1]] = [];
    else if (line === "end") cur = null;
    else if (cur && line.trim()) { const x = line.trim().match(/^(user|group) "([^"]*)"$/); cur.push([x[1], x[2]]); }
  }
  const one = (type, title) => {
    const hits = out.filter((l) => l.includes(\`<C Type="\${type}"\`) && l.includes(\` Title="\${xml(title)}"\`));
    if (hits.length !== 1) { console.error(\`error: compile error: \${hits.length} \${type} titled "\${title}"\`); process.exit(1); }
    return hits[0].match(/ U="([^"]+)"/)[1];
  };
  for (const [, slug, set] of src.matchAll(/^(\\w+)\\.recipients = (\\w+)$/gm)) {
    const n = slugs[slug];
    const want = rsets[set].map(([k, p]) => one(k === "user" ? "User" : "UserGroup", p));
    out = out.flatMap((l) => {
      if (!l.includes(\`<PRM REF="\${n}"\`)) return [l];
      const kept = l.replace(/ ACT="[^"]*"/, "");
      return / TIM="/.test(kept) ? [kept] : [];
    });
    for (const who of want) {
      const i = out.findIndex((l) => l.includes(\` U="\${who}"\`) && l.includes("<C "));
      const j = out.findIndex((l, k) => k > i && l.includes("</PH>"));
      out.splice(j, 0, \`\\t\\t\\t\\t\\t<PRM REF="\${n}" ACT="3"/>\`);
    }
  }
}
// Tiles (#78): each \`tiles\` set of vislox's file (as lox's formatter writes it: a \`tile\` on one line, or over several
// up to its \`)\`), written onto the devices its claims name as lox writes them: per function \`api\`, \`cT\`, \`bT\` and
// \`ctrls\` (the targets' UUIDs), an empty function without them; a tile the set leaves out stays.
{
  const tsets = {};
  let cur = null, buf = null;
  for (const line of src.split("\\n")) {
    const m = line.match(/^tiles (\\w+)$/);
    if (m) { cur = tsets[m[1]] = []; continue; }
    if (line === "end") { cur = null; continue; }
    if (!cur) continue;
    const t = line.trim();
    if (buf != null) { buf += " " + t; if (t === ")") { cur.push(buf); buf = null; } continue; }
    if (/^tile \\d+\\(.*\\)$/.test(t)) cur.push(t);
    else if (/^tile \\d+\\($/.test(t)) buf = t;
  }
  const FN = ["click", "up", "down"];
  for (const [, slug, set] of src.matchAll(/^(\\w+)\\.tiles = (\\w+)$/gm)) {
    const dev = slugs[slug];
    for (const t of tsets[set]) {
      const index = t.match(/^tile (\\d+)\\(/)[1];
      const cmd = {}, text = {}, to = {}, toAll = [];
      let textAll = null;
      for (const [, k, v] of t.matchAll(/(\\w+): ("(?:[^"\\\\]|\\\\.)*"|\\w+)/g)) {
        const val = v.startsWith('"') ? JSON.parse(v) : v;
        if (FN.includes(k)) cmd[k] = val;
        else if (k === "text") textAll = val;
        else if (k === "to") toAll.push(val);
        else if (k.endsWith("_text")) text[k.slice(0, -5)] = val;
        else if (k.endsWith("_to")) (to[k.slice(0, -3)] ||= []).push(val);
      }
      const i = out.findIndex((l) => l.includes(\`U="\${dev}"\`) && l.includes("<C "));
      const b = out.findIndex((l, n) => n > i && l.includes("<btn ") && l.includes(\` i="\${index}"\`));
      if (b < 0) { console.error(\`error: compile error: tile \${index}: the device has no such tile\`); process.exit(1); }
      FN.forEach((f, n) => {
        const j = out.findIndex((l, x) => x > b && l.trim().startsWith(\`<f\${n + 1}\`));
        const ind = out[j].match(/^\\s*/)[0], bt = n ? \` bT="\${n}"\` : "";
        if (!cmd[f]) { out[j] = n ? \`\${ind}<f\${n + 1}\${bt}/>\` : \`\${ind}<f1></f1>\`; return; }
        const tx = text[f] ?? textAll, ts = to[f] ?? toAll;
        out[j] = \`\${ind}<f\${n + 1} api="\${xml(cmd[f])}"\${tx ? \` cT="\${xml(tx)}"\` : ""}\${bt}\${ts.length ? \` ctrls="\${ts.map((x) => slugs[x]).join(",")}"\` : ""}/>\`;
      });
    }
  }
}
// vislox's wires (to.Port <- from.Port) between the objects its externs pin: an <In> in the sink's
// connector, from the source's connector, as lox draws it.
const METER_KEYS = { Power: "Pf", ReadingIn: "Mr", Reset: "R", ReadingOffset: "Mro" };
const coOf = (uuid, name) => {
  const i = out.findIndex((l) => l.includes(\`U="\${uuid}"\`));
  for (const k of [name, KEYS[name], METER_KEYS[name], name.replace(/^In(\\d)$/, "I$1")]) {
    const j = out.findIndex((l, n) => n > i && l.includes(\`<Co K="\${k}"\`));
    if (j > i && !out.slice(i + 1, j).some((l) => /<C Type=/.test(l))) return j;
  }
  return -1;
};
for (const line of src.split("\\n")) {
  const m = line.match(/^(\\w+)\\.(\\w+) <- (\\w+)\\.(\\w+)$/);
  if (!m) continue;
  const to = coOf(slugs[m[1]] || objects[m[1]]?.uuid, m[2]), from = coOf(slugs[m[3]] || objects[m[3]]?.uuid, m[4]);
  const srcU = out[from].match(/U="([^"]+)"/)[1];
  out[to] = out[to].replace(/\\/>$/, \`><In Input="\${srcU}"/></Co>\`);
}
// Tree and Air hardware (#85): vislox's _hardware.loxl. A created device (\`kind:\`) goes below its Tree extension,
// unpaired (Serial="00000000"), with one digital input I1, its UUID kept by slug in the lock's externals and
// hardware; one the file leaves out without \`removed\` is refused, as lox refuses it. \`Statistics: on\` writes the
// StatsType its type records with (an analog sensor's 11, else 7), \`off\` removes it.
const externals = {}, hardware = {};
{
  const hw = {}, text = fs.existsSync("pages/vislox/_hardware.loxl") ? fs.readFileSync("pages/vislox/_hardware.loxl", "utf8") : "";
  for (const [slug, e] of Object.entries(lock.externals || {})) {
    if (e.matched_by === "created" && !new RegExp(\`^(\${slug} = |removed \${slug}$)\`, "m").test(text)) {
      console.error(\`error: compile error: \\\`\${slug}\\\` created the Tree device \${e.uuid}, and the source no longer declares it. To delete the device, add \\\`removed \${slug}\\\`\`);
      process.exit(1);
    }
  }
  for (const line of text.split("\\n")) {
    let m;
    if ((m = line.match(/^extern (vx_\\w+) = LoxTree\\(uuid: "([^"]+)"\\)$/))) { hw[m[1]] = m[2]; continue; }
    if (!(m = line.match(/^(vx_\\w+) = (\\w+)\\((.*)\\)$/))) continue;
    const [, slug, ty, rest] = m;
    const kind = rest.match(/kind: "(\\w+)"/), byU = rest.match(/uuid: "([^"]+)"/);
    if (kind) {
      const tree = hw[rest.match(/tree: (\\w+)/)[1]];
      const uuid = lock.externals?.[slug]?.uuid || \`ffffff\${String(++n).padStart(2, "0")}-0000-0000-ffff504f94000001\`;
      const title = rest.match(/^"((?:[^"\\\\]|\\\\.)*)"/);
      const i = out.findIndex((l) => l.includes(\`U="\${tree}"\`) && l.includes("<C "));
      out.splice(i + 1, 0, \`\\t\\t\\t<C Type="TreeDevice" TId="\${kind[1]}" V="178" U="\${uuid}" Title="\${xml(title ? JSON.parse('"' + title[1] + '"') : kind[1])}" WF="16384" Serial="00000000">\`,
        \`\\t\\t\\t\\t<C Type="TreeSensor" IName="I1" V="178" U="\${uuid.slice(0, 9)}0000-0001-ffff504f94000001" Title="Motion" Nio="2"><Co K="Q" U="\${uuid.slice(0, 9)}0000-0002-ffff504f94000001"/></C>\`, "\\t\\t\\t</C>");
      hw[slug] = uuid;
      externals[slug] = { uuid, matched_by: "created", owned: true };
      hardware[slug] = uuid;
    } else if (byU && !rest.includes("device:")) {
      hw[slug] = byU[1];
      externals[slug] = { uuid: byU[1], matched_by: "uuid", owned: true };
    } else {
      const dev = hw[rest.match(/device: (\\w+)/)[1]], iname = rest.match(/iname: "(\\w+)"/);
      const d = out.findIndex((l) => l.includes(\`U="\${dev}"\`) && l.includes("<C "));
      const i = byU ? out.findIndex((l) => l.includes(\`U="\${byU[1]}"\`) && l.includes("<C ")) : out.findIndex((l, x) => x > d && l.includes(\`IName="\${iname[1]}"\`));
      const st = rest.match(/Statistics: (on|off)/);
      if (st) out[i] = out[i].replace(/ StatsType="[^"]*"/, "").replace(/^(\\s*<C [^>]*?)(\\/?>)/, st[1] === "on" ? \`$1 StatsType="\${ty.includes("Asensor") ? 11 : 7}"$2\` : "$1$2");
      externals[slug] = { uuid: out[i].match(/U="([^"]+)"/)[1], matched_by: byU ? "uuid" : "iname", owned: true };
    }
  }
}
// Automatic Designer rules (#87): a \`rule <slug>\` declaration of edits.loxl rewrites the rule's APC= as the app
// writes it (\`active\`, \`when weekday …\`, \`when time HH:MM\`, \`or\`, \`then <push button> on|off\`), and STA= follows
// \`active\`. A named one (\`rule vx_rule_<n> "Name"\`) creates the rule below the last one, its UUID pinned in the
// lock's \`rules\`.
const rules = {};
{
  const DAYS = { mon: [0], tue: [1], wed: [2], thu: [3], fri: [4], sat: [5], sun: [6], weekdays: [0, 1, 2, 3, 4], weekend: [5, 6], daily: [0, 1, 2, 3, 4, 5, 6] };
  for (const [, slug, title, body] of src.matchAll(/^rule (\\w+)(?: "([^"]*)")?\\n((?:\\t.*\\n)*)end$/gm)) {
    let uuid = slugs[slug];
    if (title !== undefined) {
      uuid = lock.rules?.[slug] || \`a7ffff\${String(++n).padStart(2, "0")}-0000-0000-ffff504f94000001\`;
      rules[slug] = uuid;
      if (!out.some((l) => l.includes(\`U="\${uuid}"\`))) {
        const last = out.findLastIndex((l) => l.includes('Type="AutopilotRule"'));
        const end = out.findIndex((l, i) => i > last && l.trim() === "</C>");
        out.splice(end + 1, 0, \`\\t\\t\\t<C Type="AutopilotRule" V="178" U="\${uuid}" Title="\${xml(title)}" Nio="1" WF="16384" APC="" ref="${AUTOPILOT}">\`, \`\\t\\t\\t\\t<Co K="Q" U="\${uuid.slice(0, 9)}0000-0001-ffff504f94000001"/>\`, "\\t\\t\\t</C>");
      }
    }
    let active = false;
    const groups = [{ operator: 1, events: [] }], actions = [];
    for (const raw of body.split("\\n")) {
      const l = raw.trim();
      let m;
      if (l === "active" || l === "inactive") active = l === "active";
      else if (l === "or") groups.push({ operator: 1, events: [] });
      else if ((m = l.match(/^when weekday (.*)$/))) groups.at(-1).events.push({ type: 3, descID: 270001, id: 270, value: 9999999999, operator: 1, forceEdge: false, arrValues: [...new Set(m[1].split(" ").flatMap((d) => DAYS[d]))].sort() });
      else if ((m = l.match(/^when time (\\d\\d):(\\d\\d)$/))) groups.at(-1).events.push({ type: 5, descID: 272001, id: 272, value: +m[1] * 60 + +m[2], operator: 1, forceEdge: false });
      else if ((m = l.match(/^then (\\w+) "?(on|off)"?$/))) actions.push({ type: 0, descID: m[2] === "on" ? 342101 : 342102, id: slugs[m[1]], command: m[2] });
      else if (l) { console.error(\`error: compile error: \\\`rule \${slug}\\\`: \\\`\${l}\\\`: the stand-in does not write this\`); process.exit(1); }
    }
    const i = out.findIndex((l) => l.includes(\`U="\${uuid}"\`) && l.includes("<C "));
    const name = out[i].match(/ Title="([^"]*)"/)[1];
    const payload = { type: 1, id: uuid, active, activeIfValid: active, name, description: "", operator: 3, invalid: false, eventGroups: groups, actions };
    out[i] = out[i].replace(/ APC="[^"]*"/, \` APC="\${JSON.stringify(payload).replace(/&/g, "&amp;").replace(/"/g, "&quot;")}"\`).replace(/ STA="1"/, "").replace(/ ref=/, active ? ' STA="1" ref=' : " ref=");
  }
}
const hwLock = { ...(Object.keys(externals).length ? { externals } : {}), ...(Object.keys(hardware).length ? { hardware } : {}), ...(Object.keys(rules).length ? { rules } : {}), ...(Object.keys(pages).length ? { pages } : {}) };
fs.writeFileSync(lockFile, JSON.stringify(removed ? { objects, removed, ...hwLock } : { objects, ...hwLock }));
if (lockAt < 0) fs.writeFileSync(mintFile, String(n));
const to = process.argv.indexOf("--out");
fs.writeFileSync(to > 0 ? process.argv[to + 1] : "out.Loxone", out.join("\\n"));
`);
chmodSync(fakeLox, 0o755);
const token = join(loxDir, "token");
writeFileSync(token, "secret\n");
const loxConfig = join(loxDir, "config.yaml");
writeFileSync(loxConfig, "host: http://127.0.0.1:9\nuser: u\npass: p\nserial: 504F94000001\n");

const port = 7980 + Math.floor(Math.random() * 9);
const server = spawn(bin, [dir, "--listen", `127.0.0.1:${port}`, "--lox", fakeLox, "--admin-token-file", token, "--no-live"], {
  stdio: ["ignore", "ignore", "pipe"],
  env: { ...process.env, LOX_CONFIG: loxConfig, VISLOX_CACHE: join(root, "cache") },
});
let log = "";
server.stderr.on("data", (d) => (log += d));
await new Promise((ok, fail) => {
  const t = setTimeout(() => fail(new Error("vislox did not start:\n" + log)), 15000);
  server.stderr.on("data", () => { if (log.includes("open http")) { clearTimeout(t); ok(); } });
  server.on("exit", (c) => fail(new Error(`vislox exited ${c}:\n${log}`)));
});

const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH, args: ["--no-sandbox"] } : {});
const errors = [];
let failed = false;
let page;
try {
  page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  page.on("pageerror", (e) => errors.push(e.message));
  // A refused edit answers 422 (the browser logs it as a failed request); anything else is an error.
  page.on("console", (m) => { if (m.type() === "error" && !/status of 422/.test(m.text())) errors.push(m.text()); });
  const edits = [];
  // Each edit request, and the status its response came back with (by request: a response event may
  // reach this script after the next request's).
  const reqs = [], statuses = [];
  page.on("request", (r) => { if (new URL(r.url()).pathname === "/api/edit") { reqs.push(r); edits.push(JSON.parse(r.postData())); } });
  page.on("response", (r) => { const i = reqs.indexOf(r.request()); if (i >= 0) statuses[i] = r.status(); });
  /** Wait in this script until `ok()` holds. */
  const until = async (ok, what, ms = 20000) => {
    for (const end = Date.now() + ms; !ok();) {
      if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
      await new Promise((r) => setTimeout(r, 50));
    }
  };
  page.on("dialog", (d) => d.accept());
  await page.addInitScript(() => localStorage.setItem("vislox.token", "secret"));
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForSelector(".tab");
  const step = async (name, fn) => { await fn(); console.log("ok -", name); };
  const pick = async (uuid) => {
    await page.evaluate((u) => { const pg = S.objPage.get(u); if (pg) gotoPage(pg, { focus: u }); select(u); }, uuid);
    await page.waitForFunction((u) => S.sel === u && document.querySelector('[data-body="Edit"]'), uuid);
  };
  const field = (f) => `#props [data-ed-field="${f}"]`;
  // What the page shows once it settles: an edit's reply, the History and /api/edits come one after another.
  const waitText = (sel, re) => page.waitForFunction(([s, src]) => new RegExp(src).test(document.querySelector(s)?.textContent || ""), [sel, re.source], { timeout: 10000 });
  // Undo and redo (#62): the key, then the one request it sent, once the page settled on what it
  // compiled (`done`: what the page shows then).
  // Settled: no undo or redo queued or sent, nothing compiling on the server, the newest config shown,
  // and Properties not waiting for a compile.
  const SETTLED = `!UNDO.busy && !UNDO.inflight && !showingWhy() && !S.hist?.actions?.running && S.viewRev === S.rev && S.rev === editRev()?.id &&
    !/Compiling…|Undo or redo is compiling/.test(document.querySelector("#props")?.textContent || "")`;
  const settled = () => page.waitForFunction(SETTLED, null, { timeout: 20000 });
  // The view of a pending revision, held until `release()` or answered with a 500 (#126): the check's answer then
  // comes before it, or the view never comes. `stop()` puts the route away (and releases what is held).
  const pendingView = (r) => decodeURIComponent(new URL(r.request().url()).searchParams.get("rev") || "").startsWith("pending:");
  const viewRoute = async (mode) => {
    let release;
    const held = new Promise((r) => { release = r; });
    const route = { failed: 0 };
    await page.route("**/api/view*", async (r) => {
      if (!pendingView(r)) return r.continue().catch(() => {});
      if (mode === "fail") return route.failed++, r.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "refused for the test" }) });
      await held;
      await r.continue().catch(() => {});
    });
    return Object.assign(route, { release, stop: async () => { release(); await page.unroute("**/api/view*"); } });
  };
  // The block buttons of the Deploy pane, by their text.
  const goLabels = (sel) => page.evaluate((s) => [...document.querySelectorAll(`${s} .go`)].map((b) => b.textContent), sel);
  const undoKey = async (redo, done, arg) => {
    const n = edits.length;
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press(redo ? "Control+Shift+z" : "Control+z");
    await until(() => edits.length > n && statuses[n] !== undefined, `the ${redo ? "redo" : "undo"} request`);
    await page.waitForFunction(done, arg, { timeout: 20000 });
    await settled();
    assert.equal(edits.length, n + 1, "one request");
    assert.equal(statuses[n], 200);
    return edits[n].ops;
  };


  await step("nothing is pending, and Properties offers to edit the Divide block", async () => {
    await page.waitForSelector("#deploy-pane:not(.hidden)");
    await waitText("#deploy-pend", /Nothing pending\. Edit values in Properties/);
    await pick(DIV);
    await page.click('#props [data-ed="open"]');
    // Its name and its divisor; the dividend is wired.
    assert.deepEqual(await page.$$eval("#props [data-ed-field]", (n) => n.map((x) => x.dataset.edField)), ["title", "room", "p:Input2"]);
    assert.equal(await page.inputValue(field("room")), "Küche");
    assert.equal(await page.inputValue(field("p:Input2")), "1000");
  });
  await step("the blocks the module manages are marked, with the line that declares them (#64)", async () => {
    await page.waitForFunction((u) => S.edits?.sources?.[u], FLUR);
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, FLUR);
    await page.waitForSelector(`.blk.managed[data-uuid="${FLUR}"] .mbadge`);
    // The Monoflop beside it is Loxone Config's.
    assert.equal(await page.$(`.blk[data-uuid="${MONO}"].managed`), null);
    await page.waitForSelector('[data-body="Lox Logic"]');
    assert.match(await page.textContent('[data-body="Lox Logic"]'), /flur[\s\S]*pages\/flur\.loxl:3[\s\S]*Declared on this line/);
    assert.match(await page.textContent("#props .ph-facts"), /λ flur/);
    // Its values are edited in that file, which Properties names.
    await waitText('[data-body="Edit"]', /change its values in pages\/flur\.loxl:3/);
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: `http://127.0.0.1:${port}` });
    // A double click copies twice and still puts the label back.
    // On the one button node held here: a refresh of Properties (the check or the edits arriving) may replace the
    // button meanwhile, and the label is then set on the node that was replaced.
    await page.evaluate(() => {
      window.__copy = document.querySelector('[data-body="Lox Logic"] button.copy');
      window.__copy.click(); window.__copy.click();
    });
    await page.waitForFunction(() => window.__copy.textContent === "Copied");
    assert.equal(await page.evaluate(() => navigator.clipboard.readText()), "pages/flur.loxl:3");
    await page.waitForFunction(() => window.__copy.textContent === "Copy path");
    // A block the lock pins that no .loxl file declares: its slug, and why there is no line.
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, lagerUuid(3));
    await waitText('[data-body="Lox Logic"]', /lager_3[\s\S]*no \.loxl file declares it now/);
  });
  await step("the legend fades the blocks the module does not manage", async () => {
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(null); }, FLUR);
    await page.waitForSelector("#legend.managed #chk-only-managed", { state: "visible" });
    await page.check("#chk-only-managed");
    await page.waitForSelector("#canvas.only-managed");
    const op = (u) => page.$eval(`.blk[data-uuid="${u}"]`, (n) => +getComputedStyle(n).opacity);
    assert.ok((await op(MONO)) < 0.5, "the Monoflop is faded");
    assert.equal(await op(FLUR), 1);
    await page.uncheck("#chk-only-managed");
    await page.waitForFunction(() => !document.querySelector("#canvas.only-managed"));
  });
  await step("Ctrl+K finds a managed block by its slug", async () => {
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press("Control+k");
    await page.fill("#pal-input", "lager_12");
    await page.waitForSelector("#pal-list [role=option][aria-selected=true] .pal-slug mark");
    assert.match(await page.textContent("#pal-list [role=option][aria-selected=true]"), /Lager 12[\s\S]*lager_12/);
    await page.press("#pal-input", "Enter");
    await page.waitForFunction((u) => S.sel === u, lagerUuid(12));
  });
  await step("a line moved in the module, not compiled, is followed", async () => {
    writeFileSync(join(dir, "pages", "flur.loxl"), '# The module\'s own source.\npage "Obergeschoss"\n\n# Flur\nflur = And("Flur")\n');
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, FLUR);
    await waitText('[data-body="Lox Logic"]', /pages\/flur\.loxl:5/);
    // Back to the Divide block, whose form is still open, for the steps that follow.
    await pick(DIV);
    await page.waitForSelector(field("p:Input2"));
  });
  await step("a value that is not a number is refused in the browser", async () => {
    await page.fill(field("p:Input2"), "1,5");
    await page.click('#props [data-ed="save"]');
    await page.waitForSelector(`${field("p:Input2")}[aria-invalid="true"]`);
    assert.match(await page.textContent("#props .ed-bad"), /decimal point/);
    assert.equal(edits.length, 0);
  });
  await step("saving compiles the edit and shows it as pending, not deployed", async () => {
    await page.fill(field("title"), "Teiler");
    await page.selectOption(field("room"), "Bad");
    await page.fill(field("p:Input2"), "100");
    await page.press(field("p:Input2"), "Enter");
    await page.waitForFunction(() => S.rev?.startsWith("pending:") && S.view?.objects["10000006-0000-0060-ffff504f94000001"]?.title === "Teiler", null, { timeout: 10000 });
    assert.equal(edits.length, 1);
    assert.deepEqual(edits[0].ops, [
      { op: "title", uuid: DIV, value: "Teiler" },
      { op: "room", uuid: DIV, value: "Bad" },
      { op: "param", uuid: DIV, port: "Input2", value: "100" },
    ]);
    await page.waitForFunction(() => document.querySelectorAll("#props .ed-tag").length === 3);
    assert.equal(await page.evaluate((u) => S.view.objects[u].room, DIV), "Bad");
    await waitText("#props", /Saved and compiled: pending, not deployed/);
    await waitText("#deploy-pend", /not deployed\. They include 3 values edited here/);
    await page.waitForSelector("#btn-apply:not([disabled])");
    await waitText(".changes", /Teiler[\s\S]*1000 → 100|1000 → 100[\s\S]*Teiler/);
    const file = readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8");
    assert.match(file, /^extern vx_10000006_0000_0060_ffff504f94000001 = Div\(uuid: "10000006-0000-0060-ffff504f94000001"\)$/m);
    assert.match(file, /\.In2 = 100$/m);
  });
  await step("undo puts the values back as a new edit, redo makes it again", async () => {
    await page.waitForSelector("#btn-undo:not([disabled])");
    assert.match(await page.getAttribute("#btn-undo", "title"), /^Undo: 3 values of .+ \((Ctrl\+Z|⌘Z)\)/);
    assert.equal(await page.isDisabled("#btn-redo"), true);
    let ops = await undoKey(false, (u) => S.view?.objects[u]?.title !== "Teiler" && S.view.objects[u].room === "Küche", DIV);
    // The reply's `undo`: each op's previous, the last op's first.
    assert.deepEqual(ops, [
      { op: "param", uuid: DIV, port: "Input2", value: null },
      { op: "room", uuid: DIV, value: null },
      { op: "title", uuid: DIV, value: null },
    ]);
    assert.equal(await page.evaluate((u) => S.view.objects[u].ports.find((p) => p.key === "Input2").def, DIV), "1000");
    assert.doesNotMatch(existsSync(join(dir, "pages/vislox/edits.loxl")) ? readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8") : "", /title|In2/);
    await waitText("#deploy-msg", /Undone: 3 values of .*Compiled: pending, not deployed/);
    assert.match(await page.getAttribute("#btn-redo", "title"), /^Redo: 3 values of /);
    ops = await undoKey(true, (u) => S.view?.objects[u]?.title === "Teiler" && S.view.objects[u].room === "Bad", DIV);
    assert.deepEqual(ops, edits[0].ops);
    await page.waitForFunction(() => document.querySelectorAll("#props .ed-tag").length === 3);
    // The list of recent edits names it.
    await page.click("#btn-undo-list");
    await waitText("#undo-pop", /3 values of/);
    await page.click("#btn-undo-list");
  });
  await step("a value out of its documented range is refused by the server, and nothing changes", async () => {
    await pick(MONO);
    await page.click('#props [data-ed="open"]');
    await page.fill(field("p:Time"), "-3");
    assert.equal(await page.textContent("#props .ed-unit"), "s");
    await page.click('#props [data-ed="save"]');
    await page.waitForFunction(() => /outside the documented range/.test(document.querySelector("#props .ed-msg.err")?.textContent || ""));
    assert.doesNotMatch(readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8"), /Time/);
    await page.press(field("p:Time"), "Escape");
  });
  await step("Revert hands one value back", async () => {
    await pick(DIV);
    const n = edits.length;
    // The list is replaced while the page shows it: Revert still reverts its own value.
    await page.evaluate(() => S.edits.claims.reverse());
    await page.click('#props .pr:has-text("In2") [data-ed="revert"]');
    await page.waitForFunction(() => document.querySelectorAll("#props .ed-tag").length === 2);
    assert.deepEqual(edits[n].ops, [{ op: "param", uuid: DIV, port: "Input2", value: null }]);
    assert.doesNotMatch(readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8"), /In2/);
  });
  const saveTitle = async (t) => {
    await pick(DIV);
    await page.click('#props [data-ed="open"]');
    await page.fill(field("title"), t);
    await page.press(field("title"), "Enter");
  };
  // The Divide block named `t`, and the page settled (both at once: a name read mid-compile is no answer).
  const titleIs = (t) => page.waitForFunction(`S.view?.objects["${DIV}"]?.title === ${JSON.stringify(t)} && ${SETTLED}`, null, { timeout: 20000 });
  await step("a quick Ctrl+Z right after a save undoes that save, and quick steps wait their turn", async () => {
    const n = edits.length;
    await saveTitle("Schnell");
    // At once: the save is still compiling, and the History has not shown it.
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press("Control+z");
    await until(() => edits.length >= n + 2 && statuses[n + 1] !== undefined, "the save, then its undo");
    await titleIs("Teiler");
    assert.equal(edits.length, n + 2, "the save, then its undo");
    assert.deepEqual(edits[n + 1].ops, [{ op: "title", uuid: DIV, value: "Teiler" }]);
    // Three keys in a row: redo, undo, redo, each against the config the one before left.
    await page.keyboard.press("Control+Shift+z");
    await page.keyboard.press("Control+z");
    await page.keyboard.press("Control+Shift+z");
    await until(() => edits.length >= n + 5 && statuses[n + 4] !== undefined, "three steps");
    await titleIs("Schnell");
    assert.equal(edits.length, n + 5);
    assert.deepEqual(statuses.slice(n, n + 5), [200, 200, 200, 200, 200], "no stale base, no 409");
    await undoKey(false, (u) => S.view?.objects[u]?.title === "Teiler", DIV);
  });
  await step("an undo that changes nothing is done, and its redo kept", async () => {
    await saveTitle("Gleich");
    await titleIs("Gleich");
    // The undo is sent as a claim of what is claimed already: the server compiles nothing.
    await page.route("**/api/edit", (r) => r.continue({ postData: JSON.stringify({ ...JSON.parse(r.request().postData()), ops: [{ op: "title", uuid: DIV, value: "Gleich" }] }) }), { times: 1 });
    await page.keyboard.press("Control+z");
    await waitText("#deploy-msg", /Undone: Name of .*Nothing changed/);
    await page.waitForFunction(`/Gleich/.test(UNDO.undone.at(-1)?.label || "") && ${SETTLED}`, null, { timeout: 20000 });
    assert.equal(await page.evaluate(() => UNDO.note), "");
    // Its redo is the edit as made (the config already is that way), and the undo after it is real.
    await page.keyboard.press("Control+Shift+z");
    await page.waitForFunction(`/Gleich/.test(UNDO.done.at(-1)?.label || "") && !UNDO.undone.length && ${SETTLED}`, null, { timeout: 20000 });
    await undoKey(false, (u) => S.view?.objects[u]?.title === "Teiler", DIV);
  });
  await step("a redo whose reply has no undo clears the history and says why", async () => {
    await page.route("**/api/edit", async (r) => {
      const res = await r.fetch(), j = await res.json();
      delete j.undo;
      j.undo_error = "op 1 (title): no way back for the test";
      await r.fulfill({ response: res, json: j });
    }, { times: 1 });
    await page.keyboard.press("Control+Shift+z");
    await titleIs("Gleich");
    await page.waitForFunction(`!UNDO.done.length && !UNDO.undone.length && ${SETTLED}`, null, { timeout: 20000 });
    assert.match(await page.evaluate(() => UNDO.note), /cannot be undone: op 1 \(title\): no way back for the test\. The undo history was cleared\./);
    await waitText("#deploy-msg", /cannot be undone/);
    await page.waitForSelector("#btn-undo[disabled]", { state: "attached" });
    // Back to the name the later steps expect: an edit of its own.
    await saveTitle("Teiler");
    await titleIs("Teiler");
  });
  await step("Add block offers the types lox can create, and checks values in the browser", async () => {
    await page.evaluate((u) => gotoPage(u), OG);
    await page.waitForSelector("#btn-add-block:not([disabled])");
    await page.click("#btn-add-block");
    await page.waitForSelector("#ab-q");
    assert.equal(await page.textContent("#ab-page"), "on Obergeschoss");
    // Virtual inputs are not blocks of a page.
    await page.fill("#ab-q", "virtual in");
    assert.doesNotMatch(await page.textContent("#ab-list"), /VirtualIn\b/);
    await page.fill("#ab-q", "monofl");
    assert.match(await page.textContent("#ab-list [aria-selected=true]"), /Monoflop/);
    await page.press("#ab-q", "Enter");
    await page.waitForSelector('#ab-body [data-ab-field="title"]');
    const fields = await page.$$eval("#ab-body [data-ab-field]", (n) => n.map((x) => x.dataset.abField));
    assert.ok(fields.includes("p:Time") && fields.includes("p:InputTrigger"), fields.join(" "));
    await page.fill('#ab-body [data-ab-field="title"]', "Nachlauf");
    await page.fill('#ab-body [data-ab-field="p:Time"]', "1,5");
    await page.click('#ab-body [data-ab="add"]');
    await page.waitForSelector('#ab-body [data-ab-field="p:Time"][aria-invalid="true"]');
    assert.match(await page.textContent("#ab-body .ed-bad"), /decimal point/);
  });
  let added;
  await step("adding compiles it into the pending revision, on the page, not deployed", async () => {
    const n = edits.length;
    await page.fill('#ab-body [data-ab-field="p:Time"]', "30");
    await page.press('#ab-body [data-ab-field="p:Time"]', "Enter");
    await page.waitForFunction(() => S.sel && S.view?.objects[S.sel]?.title === "Nachlauf", null, { timeout: 10000 });
    assert.equal(await page.isVisible("#addblk"), false);
    assert.deepEqual(edits[n].ops, [{ op: "add_block", page: OG, type: "Monoflop", title: "Nachlauf", params: { Time: "30" }, settings: {} }]);
    added = await page.evaluate(() => S.sel);
    assert.equal(await page.evaluate((u) => S.objPage.get(u), added), OG);
    const file = readFileSync(join(dir, "pages/vislox/obergeschoss.loxl"), "utf8");
    assert.match(file, /^page "Obergeschoss"$/m);
    assert.match(file, /^vx_monoflop_1 = Monoflop\("Nachlauf", Time: 30\)$/m);
    await waitText("#props", /added here/);
    await waitText("#props", /Added and compiled: pending, not deployed/);
    await waitText("#deploy-pend", /1 block added here/);
    await waitText(".changes", /Nachlauf/);
  });
  await step("undo takes the new block out, redo adds it again under a new UUID", async () => {
    let ops = await undoKey(false, (u) => S.view && !S.view.objects[u], added);
    assert.deepEqual(ops, [{ op: "remove_block", uuid: added }]);
    assert.match(await page.getAttribute("#btn-redo", "title"), /^Redo: Add Nachlauf/);
    ops = await undoKey(true, () => Object.values(S.view?.objects || {}).some((o) => o.title === "Nachlauf"));
    assert.equal(ops[0].op, "add_block");
    assert.equal(ops[0].restores, added);
    assert.deepEqual(ops[0].params, { Time: "30" });
    added = await page.evaluate(() => Object.values(S.view.objects).find((o) => o.title === "Nachlauf").uuid);
    await pick(added);
    await waitText("#props", /added here/);
  });
  await step("a block added here is edited like any other", async () => {
    await page.click('#props [data-ed="open"]');
    // Its declaration takes the name and constants; lox has no room or category for it.
    assert.deepEqual(await page.$$eval("#props [data-ed-field]", (n) => n.map((x) => x.dataset.edField)), ["title", "p:Time"]);
    await page.fill(field("p:Time"), "45");
    await page.press(field("p:Time"), "Enter");
    await page.waitForFunction((u) => S.view?.objects[u]?.ports.find((p) => p.key === "Time")?.def === "45", added, { timeout: 10000 });
    await waitText("#props", /Saved and compiled: pending, not deployed/);
    assert.match(readFileSync(join(dir, "pages/vislox/obergeschoss.loxl"), "utf8"), /Time: 45\)$/m);
  });
  await step("Remove takes it out again", async () => {
    const n = edits.length;
    await page.click('#props [data-ed="remove"]');
    await page.waitForFunction((u) => S.view && !S.view.objects[u], added, { timeout: 10000 });
    assert.deepEqual(edits[n].ops, [{ op: "remove_block", uuid: added }]);
    const file = readFileSync(join(dir, "pages/vislox/obergeschoss.loxl"), "utf8");
    assert.match(file, /^removed vx_monoflop_1$/m);
    assert.doesNotMatch(file, /= Monoflop/);
    await page.waitForFunction(() => !/added here/.test(document.querySelector("#deploy-pend")?.textContent || ""));
  });
  await step("undo adds the removed block again, and later undos follow its new UUID", async () => {
    let ops = await undoKey(false, () => Object.values(S.view?.objects || {}).some((o) => o.title === "Nachlauf"));
    assert.equal(ops[0].op, "add_block");
    assert.equal(ops[0].restores, added);
    const again = await page.evaluate(() => Object.values(S.view.objects).find((o) => o.title === "Nachlauf").uuid);
    assert.notEqual(again, added, "lox minted a new UUID");
    assert.equal(await page.evaluate((u) => S.view.objects[u].ports.find((p) => p.key === "Time").def, again), "45");
    // The edit of Time made before the removal now names the new UUID.
    ops = await undoKey(false, (u) => S.view?.objects[u]?.ports.find((p) => p.key === "Time")?.def === "30", again);
    assert.deepEqual(ops, [{ op: "param", uuid: again, port: "Time", value: "30" }]);
    ops = await undoKey(true, (u) => S.view?.objects[u]?.ports.find((p) => p.key === "Time")?.def === "45", again);
    assert.deepEqual(ops, [{ op: "param", uuid: again, port: "Time", value: "45" }]);
    ops = await undoKey(true, (u) => S.view && !S.view.objects[u], again);
    assert.deepEqual(ops, [{ op: "remove_block", uuid: again }]);
    assert.match(readFileSync(join(dir, "pages/vislox/obergeschoss.loxl"), "utf8"), /^removed vx_monoflop_\d+$/m);
  });
  // Moving (#60): lox keeps a managed block's position in the lock; vislox writes it there and compiles.
  const flurNow = () => page.evaluate((u) => curPage()?.objects.find((o) => o.uuid === u && !o.moved_to), FLUR);
  const flurLock = () => JSON.parse(readFileSync(join(dir, "logic.lock.json"), "utf8")).objects.flur;
  // A newer pending revision than `since`, once it is shown and draws the block at `x`, marked moved or not
  // (while a move is sent, the page already draws the block where it was dropped).
  const rev = () => page.evaluate(() => S.rev);
  const flurAt = (since, x, moved) => page.waitForFunction(([u, since, x, moved]) => {
    const o = curPage()?.objects.find((o) => o.uuid === u && !o.moved_to);
    return S.rev !== since && S.viewRev === S.rev && S.rev?.startsWith("pending:") && !MV && !S.mvBusy &&
      o?.rect[0] === x && (o.status === "moved") === moved;
  }, [FLUR, since, x, moved], { timeout: 10000 });
  let moved;
  await step("a block the module manages is dragged on the page, snapped to the grid, and shows as moved", async () => {
    await settled();
    // In the middle of the canvas, where nothing covers it.
    await page.evaluate(([pg, u]) => { gotoPage(pg); focusObj(u, 0.05); }, [OG, FLUR]);
    const g = page.locator(`#canvas g.movable[data-uuid="${FLUR}"]`).first();
    await g.waitFor();
    const n = edits.length, r0 = await rev();
    const box = await (await page.waitForFunction(([pg, u]) => {
      gotoPage(pg); focusObj(u, 0.05);
      const r = document.querySelector(`#canvas g.movable[data-uuid="${u}"]`)?.getBoundingClientRect();
      return r?.width && r.height ? { x: r.x, y: r.y, width: r.width, height: r.height } : false;
    }, [OG, FLUR])).jsonValue();
    const [cx, cy] = [box.x + box.width / 2, box.y + box.height / 2];
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 80, cy + 50, { steps: 8 });
    await page.mouse.up();
    await page.waitForFunction((k) => document.querySelectorAll("#props .ed-tag").length > 0 && S.sel === k, FLUR, { timeout: 10000 });
    assert.equal(edits.length, n + 1);
    const op = edits[n].ops[0];
    assert.equal(op.op, "move");
    assert.equal(op.uuid, FLUR);
    assert.ok(op.x % 96 === 0 && op.y % 96 === 0 && (op.x !== 7104 || op.y !== 2880), JSON.stringify(op));
    moved = op;
    await flurAt(r0, op.x, true);
    assert.equal((await flurNow()).rect[1], op.y);
    // In the lock, with the position the base still draws superseded; vislox's files claim nothing of it.
    const l = flurLock();
    assert.deepEqual([l.layout.px, l.layout.py, l.layout.px2 - l.layout.px], [op.x, op.y, 8448 - 7104]);
    assert.equal(l.superseded_layout.px, 7104);
    assert.doesNotMatch(readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8"), /1000000b/);
    await waitText("#props", /Position\s*moved\s*not deployed/);
    await waitText("#deploy-pend", /1 block moved here/);
    await page.waitForSelector("#canvas .ghost");
  });
  await step("Alt and an arrow key nudge it one grid step", async () => {
    const n = edits.length, r0 = await rev();
    await page.keyboard.press("Alt+ArrowRight");
    await flurAt(r0, moved.x + 96, true);
    assert.equal(edits.length, n + 1);
    assert.deepEqual(edits[n].ops, [{ op: "move", uuid: FLUR, x: moved.x + 96, y: moved.y }]);
  });
  await step("undo and redo a move", async () => {
    const at = ([u, r0, x]) => S.rev !== r0 && !MV && !S.mvBusy && curPage()?.objects.find((o) => o.uuid === u && !o.moved_to)?.rect[0] === x;
    let ops = await undoKey(false, at, [FLUR, await rev(), moved.x]);
    assert.deepEqual(ops, [{ op: "move", uuid: FLUR, x: moved.x, y: moved.y }]);
    ops = await undoKey(true, at, [FLUR, await rev(), moved.x + 96]);
    assert.deepEqual(ops, [{ op: "move", uuid: FLUR, x: moved.x + 96, y: moved.y }]);
    await page.evaluate((u) => select(u), FLUR);
  });
  await step("nudged past the top of the page, it stops at the edge", async () => {
    const n = edits.length, r0 = await rev(), x = (await flurNow()).rect[0];
    for (let i = 0; i < 4; i++) await page.keyboard.press("Alt+Shift+ArrowUp");
    await flurAt(r0, x, true);
    assert.equal(edits.length, n + 1);
    assert.deepEqual(edits[n].ops, [{ op: "move", uuid: FLUR, x, y: 0 }]);
    assert.equal((await flurNow()).rect[1], 0);
  });
  await step("a refused move puts the block back; one compiled but not shown stays", async () => {
    // Settled: no move in flight, the edit's message shown.
    const settled = (re) => page.waitForFunction((src) => !MV && !S.mvBusy && new RegExp(src).test(S.edit.err), re.source, { timeout: 10000 });
    const x = (await flurNow()).rect[0];
    await page.route("**/api/edit", (r) => r.fulfill({ status: 422, contentType: "application/json", body: JSON.stringify({ error: "refused for the test" }) }), { times: 1 });
    await page.keyboard.press("Alt+ArrowLeft");
    await settled(/refused for the test/);
    assert.equal((await flurNow()).rect[0], x);
    // Compiled, but showing the new revision fails once: the canvas keeps the move.
    const r0 = await rev();
    await page.evaluate(() => { const show = editShowNewest; window.editShowNewest = async () => { window.editShowNewest = show; throw new Error("no history"); }; });
    await page.keyboard.press("Alt+ArrowLeft");
    await settled(/could not show it: no history/);
    assert.equal((await flurNow()).rect[0], x - 96);
    await page.evaluate(() => editShowNewest());
    await flurAt(r0, x - 96, true);
  });
  await step("Revert puts it back where it is deployed", async () => {
    const n = edits.length, r0 = await rev();
    await page.click('#props [data-ed="unmove"]');
    await flurAt(r0, 7104, false);
    assert.deepEqual(edits[n].ops, [{ op: "move", uuid: FLUR, x: 7104, y: 2880 }]);
    assert.equal(flurLock().superseded_layout, undefined);
    await page.waitForFunction(() => !/moved here/.test(document.querySelector("#deploy-pend")?.textContent || ""));
  });
  await step("Arrange lays out the page's managed blocks afresh", async () => {
    const n = edits.length, r0 = await rev();
    await page.waitForSelector("#btn-relayout:not([disabled])");
    await page.click("#btn-relayout");
    await flurAt(r0, 9600, true);
    assert.deepEqual(edits[n].ops, [{ op: "relayout", page: OG }]);
    assert.match(readFileSync(calls, "utf8"), /^logic compile --relayout Obergeschoss$/m);
  });
  await step("undo moves the arranged block back, redo arranges it again", async () => {
    const at = ([u, r0, x]) => S.rev !== r0 && curPage()?.objects.find((o) => o.uuid === u && !o.moved_to)?.rect[0] === x;
    let ops = await undoKey(false, at, [FLUR, await rev(), 7104]);
    assert.deepEqual(ops, [{ op: "place", moves: [{ uuid: FLUR, x: 7104, y: 2880 }] }]);
    assert.equal(flurLock().layout.px, 7104);
    ops = await undoKey(true, at, [FLUR, await rev(), 9600]);
    assert.deepEqual(ops, [{ op: "place", moves: [{ uuid: FLUR, x: 9600, y: 960 }] }]);
    assert.equal(flurLock().layout.px, 9600);
  });
  await step("Arrange on a page of 210 managed blocks is undone in one op", async () => {
    await page.evaluate((pg) => gotoPage(pg), LAGER);
    await page.waitForSelector("#btn-relayout:not([disabled])");
    const n = edits.length, r0 = await rev(), last = lagerUuid(LAGER_N - 1);
    const rectOf = (u) => page.evaluate((u) => curPage()?.objects.find((o) => o.uuid === u && !o.moved_to)?.rect.slice(0, 2), u);
    const shows = ([u, r0, x, y]) => S.rev !== r0 && S.viewRev === S.rev && S.rev === editRev()?.id && !S.mvBusy && !UNDO.busy &&
      (([a, b]) => a === x && b === y)(curPage()?.objects.find((o) => o.uuid === u && !o.moved_to)?.rect || []);
    const moved = ([u, r0, x]) => S.rev !== r0 && S.viewRev === S.rev && S.rev === editRev()?.id && !S.mvBusy &&
      curPage()?.objects.find((o) => o.uuid === u && !o.moved_to)?.rect[0] !== x;
    await page.click("#btn-relayout");
    await page.waitForFunction(moved, [last, r0, lagerAt(LAGER_N - 1).px], { timeout: 15000 });
    assert.deepEqual(edits[n].ops, [{ op: "relayout", page: LAGER }]);
    const arranged = await rectOf(last);
    let ops = await undoKey(false, shows, [last, await rev(), lagerAt(LAGER_N - 1).px, lagerAt(LAGER_N - 1).py]);
    assert.equal(ops.length, 1, "one op, not one per block");
    assert.equal(ops[0].op, "place");
    assert.equal(ops[0].moves.length, LAGER_N);
    assert.deepEqual(await rectOf(lagerUuid(7)), [lagerAt(7).px, lagerAt(7).py]);
    ops = await undoKey(true, shows, [last, await rev(), ...arranged]);
    assert.equal(ops.length, 1);
    assert.equal(ops[0].moves.length, LAGER_N);
    // Back where it was, for the steps after.
    await undoKey(false, shows, [lagerUuid(0), await rev(), lagerAt(0).px, lagerAt(0).py]);
  });
  await step("a block Loxone Config drew is not moved: dragging it pans the page", async () => {
    await page.evaluate(([pg, u]) => { gotoPage(pg); focusObj(u, 0.05); }, ["10000002-0000-0020-ffff504f94000001", DIV]);
    const g = page.locator(`#canvas g[data-uuid="${DIV}"]`).first();
    await g.waitFor();
    assert.equal(await page.locator(`#canvas g.movable[data-uuid="${DIV}"]`).count(), 0);
    // A page without managed blocks has nothing to arrange.
    assert.equal(await page.isDisabled("#btn-relayout"), true);
    const n = edits.length, vx = await page.evaluate(() => S.vp[S.page].x);
    const box = await g.boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 80, box.y + box.height / 2, { steps: 8 });
    await page.mouse.up();
    assert.notEqual(await page.evaluate(() => S.vp[S.page].x), vx);
    assert.equal(edits.length, n);
  });
  // Wires (#61): drawn by dragging from a port to a port, written as `to <- from` into vislox's file.
  const portBox = async (uuid, key) => {
    const b = await page.locator(`#canvas .port-hit[data-port="${uuid}|${key}"]`).boundingBox();
    return [b.x + b.width / 2, b.y + b.height / 2];
  };
  const dragTo = async ([x0, y0], [x1, y1]) => {
    await page.mouse.move(x0, y0);
    await page.mouse.down();
    await page.mouse.move((x0 + x1) / 2, (y0 + y1) / 2, { steps: 6 });
    await page.mouse.move(x1, y1, { steps: 6 });
    await page.mouse.up();
  };
  // A wire's ends as the page draws it, once the newest pending revision shows it (or not).
  const wireShown = (from, to, there) => page.waitForFunction(([f, t, there]) => S.viewRev === S.rev && S.rev?.startsWith("pending:") && !S.wrBusy &&
    curPage()?.wires.some((w) => w.from.join() === f.join() && w.to.join() === t.join() && w.status !== "removed") === there, [from, to, there], { timeout: 10000 });
  /** Click a wire: its hit path, halfway along it (port targets drawn over a wire's ends may cover the
   *  point a mouse would hit, so the click goes to the path itself, mouse down and up first). */
  const clickWire = async (from, to) => {
    await page.waitForFunction(() => document.querySelector("#canvas .port-hit") && !S.wrBusy, null, { timeout: 10000 });
    await page.evaluate(([f, t]) => {
      const line = [...document.querySelectorAll("#canvas path.wire")].find((l) => l.dataset.f === f[0] && l.dataset.t === t[0] && !l.classList.contains("d-removed"));
      const hit = line.nextElementSibling, len = hit.getTotalLength(), pt = hit.getPointAtLength(len / 2);
      const m = hit.getScreenCTM();
      const at = { bubbles: true, clientX: m.a * pt.x + m.e, clientY: m.d * pt.y + m.f };
      for (const t of ["mousedown", "mouseup", "click"]) hit.dispatchEvent(new MouseEvent(t, at));
    }, [from, to]);
    await page.waitForSelector("#wcard:not(.hidden) .wc-head");
  };
  const wireFile = () => existsSync(join(dir, "pages/vislox/edits.loxl")) ? readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8") : "";
  let target;
  await step("a wire is drawn by dragging from an output to an input, and shows as pending", async () => {
    await page.evaluate(([pg, u]) => { gotoPage(pg); focusObj(u, 0.05); }, [EG, DIV]);
    await page.waitForSelector(`#canvas .port-hit[data-port="${DIV}|AQ"]`);
    const n = edits.length;
    await dragTo(await portBox(DIV, "AQ"), await portBox(AND, "I2"));
    await wireShown([DIV, "AQ"], [AND, "I2"], true);
    assert.deepEqual(edits[n].ops, [{ op: "wire", from: [DIV, "AQ"], to: [AND, "I2"] }]);
    assert.match(wireFile(), /^vx_10000004_0000_0040_ffff504f94000001\.In2 <- vx_10000006_0000_0060_ffff504f94000001\.AQ$/m);
    await waitText("#deploy-pend", /1 wire drawn here/);
    await waitText(".changes", /\+Teiler\.AQ → O1001\.In2/);
    await page.waitForFunction(() => curPage().wires.some((w) => w.status === "added"));
  });
  await step("undo takes the drawn wire out, redo draws it again", async () => {
    const has = ([f, t, there]) => S.viewRev === S.rev && curPage()?.wires.some((w) => w.from.join() === f.join() && w.to.join() === t.join() && w.status !== "removed") === there;
    let ops = await undoKey(false, has, [[DIV, "AQ"], [AND, "I2"], false]);
    assert.deepEqual(ops, [{ op: "unwire", from: [DIV, "AQ"], to: [AND, "I2"] }]);
    assert.doesNotMatch(wireFile(), /<-/);
    ops = await undoKey(true, has, [[DIV, "AQ"], [AND, "I2"], true]);
    assert.deepEqual(ops, [{ op: "wire", from: [DIV, "AQ"], to: [AND, "I2"] }]);
    assert.match(wireFile(), /\.In2 <- /);
  });
  await step("a link Loxone Config would not have is refused in the browser, with the reason", async () => {
    const n = edits.length;
    // The AND gate already feeds the meter.
    await dragTo(await portBox(AND, "Q"), await portBox(METER, "Pf"));
    await waitText("#wcard .wc-msg.err", /already wired/);
    // An output onto an output is not even a target.
    assert.equal(await page.locator(`#canvas .port-hit[data-port="${METER}|Pf"]`).count(), 1);
    assert.equal(edits.length, n);
    await page.keyboard.press("Escape");
    await page.waitForSelector("#wcard.hidden", { state: "attached" });
  });
  await step("a click on a port, without a drag, selects its block", async () => {
    const n = edits.length;
    await page.evaluate(() => select(null));
    const [x, y] = await portBox(METER, "Pf");
    await page.mouse.click(x, y);
    await page.waitForFunction((u) => S.sel === u, METER);
    assert.equal(edits.length, n);
    assert.equal(await page.locator("#canvas .wire-draft").count(), 0);
  });
  await step("a drop while another edit compiles says so, and the card does not hang", async () => {
    const n = edits.length;
    const [x0, y0] = await portBox(AND, "Q"), [x1, y1] = await portBox(DIV, "Input2");
    await page.mouse.move(x0, y0);
    await page.mouse.down();
    await page.mouse.move((x0 + x1) / 2, (y0 + y1) / 2, { steps: 6 });
    await page.mouse.move(x1, y1, { steps: 6 });
    // A move starts compiling before the mouse is let go.
    await page.evaluate(() => { S.mvBusy = true; });
    await page.mouse.up();
    await waitText("#wcard .wc-msg.err", /Another edit is compiling/);
    assert.equal(edits.length, n);
    assert.equal(await page.locator("#canvas .wire-draft").count(), 0);
    await page.evaluate(() => { S.mvBusy = false; renderPage(); });
    await page.click('#wcard [data-wc="close"]');
    await page.waitForSelector("#wcard.hidden", { state: "attached" });
  });
  await step("a wire Loxone Config drew says so, and offers no Delete", async () => {
    await clickWire([AND, "Q"], [METER, "Pf"]);
    await waitText("#wcard", /Drawn in Loxone Config/);
    assert.equal(await page.locator('#wcard [data-wc="unwire"]').count(), 0);
    await page.click('#wcard [data-wc="close"]');
  });
  await step("an end of a wire drawn here is dragged to another input: reconnected in one edit", async () => {
    await clickWire([DIV, "AQ"], [AND, "I2"]);
    await waitText("#wcard", /drawn here\s*not deployed/);
    // An unwired input of the meter that the page draws.
    target = await page.evaluate((u) => {
      const o = curPage().objects.find((o) => o.uuid === u), wired = L.wired.get(u);
      return o.ports.find((q) => q.side === "L" && !wired?.has(q.key)).key;
    }, METER);
    const n = edits.length;
    const h = await page.locator('#canvas .wire-end[data-end="to"]').boundingBox();
    await dragTo([h.x + h.width / 2, h.y + h.height / 2], await portBox(METER, target));
    await wireShown([DIV, "AQ"], [METER, target], true);
    assert.deepEqual(edits[n].ops, [
      { op: "unwire", from: [DIV, "AQ"], to: [AND, "I2"] },
      { op: "wire", from: [DIV, "AQ"], to: [METER, target] },
    ]);
    await wireShown([DIV, "AQ"], [AND, "I2"], false);
    assert.doesNotMatch(wireFile(), /In2 <-/);
  });
  await step("undo of a reconnect is one edit that puts the old wire back", async () => {
    const has = ([f, t, there]) => S.viewRev === S.rev && curPage()?.wires.some((w) => w.from.join() === f.join() && w.to.join() === t.join() && w.status !== "removed") === there;
    assert.match(await page.getAttribute("#btn-undo", "title"), /^Undo: Reconnect /);
    let ops = await undoKey(false, has, [[DIV, "AQ"], [AND, "I2"], true]);
    assert.deepEqual(ops, [
      { op: "unwire", from: [DIV, "AQ"], to: [METER, target] },
      { op: "wire", from: [DIV, "AQ"], to: [AND, "I2"] },
    ]);
    await wireShown([DIV, "AQ"], [METER, target], false);
    ops = await undoKey(true, has, [[DIV, "AQ"], [METER, target], true]);
    assert.deepEqual(ops, [
      { op: "unwire", from: [DIV, "AQ"], to: [AND, "I2"] },
      { op: "wire", from: [DIV, "AQ"], to: [METER, target] },
    ]);
    await wireShown([DIV, "AQ"], [AND, "I2"], false);
  });
  await step("an open wire card and a caller waiting for the edits follow the newest load (#141)", async () => {
    // The card is drawn from /api/edits: answered with a stale list (no wire drawn here), it offers no Revert; the
    // true answer comes after, and the card has to follow it (it was a snapshot: Delete then did nothing).
    let stale = true;
    await page.route("**/api/edits", async (r) => {
      if (!stale) return r.continue().catch(() => {});
      const resp = await r.fetch(), j = await resp.json();
      await r.fulfill({ response: resp, json: { ...j, wires: [] } }).catch(() => {});
    });
    try {
      await page.evaluate(() => { S.editsKey = "stale"; editsChanged(); return S.editsLoad; });
      await clickWire([DIV, "AQ"], [METER, target]);
      assert.equal(await page.locator('#wcard [data-wc="unwire"]').count(), 0);
      stale = false;
      await page.evaluate(() => { S.editsKey = "fresh"; editsChanged(); return S.editsLoad; });
      await page.waitForSelector('#wcard [data-wc="unwire"]:not([disabled])', { timeout: 5000 });
      // The edit state changes (as every History refresh may): the button is patched in place, never replaced.
      assert.deepEqual(await page.evaluate(() => {
        const b = document.querySelector('#wcard [data-wc="unwire"]'), out = [];
        S.mvBusy = true; renderAddHud();
        out.push(b.isConnected && b === document.querySelector('#wcard [data-wc="unwire"]') && b.disabled, /compiling/.test(wcard.querySelector(".wc-why").textContent));
        S.mvBusy = false; renderAddHud();
        out.push(b.isConnected && b === document.querySelector('#wcard [data-wc="unwire"]') && !b.disabled, wcard.querySelector(".wc-why").textContent);
        return out;
      }), [true, true, true, ""]);
      // A wire the page does not draw any more closes the card.
      assert.deepEqual(await page.evaluate(() => { S.wsel = { from: ["gone", "X"], to: ["gone", "Y"], at: [8, 8] }; wcardRefresh(); return [S.wsel, wcard.classList.contains("hidden")]; }), [null, true]);
    } finally { await page.unroute("**/api/edits"); }
    // Two loads overlap, the second held: whoever waits for the first waits for the second's data.
    let n = 0, release;
    const held = new Promise((r) => { release = r; });
    await page.route("**/api/edits", async (r) => {
      if (++n < 2) return r.continue().catch(() => {});
      await held;
      await r.continue().catch(() => {});
    });
    try {
      await page.evaluate(() => { S.editsKey = "a"; editsChanged(); window.__a = S.editsLoad; S.editsKey = "b"; editsChanged(); });
      assert.equal(await page.evaluate(() => Promise.race([window.__a.then(() => "done"), new Promise((r) => setTimeout(() => r("waiting"), 1000))])), "waiting");
      release();
      assert.equal(await page.evaluate(() => window.__a.then(() => "done")), "done");
    } finally {
      release();
      await page.unroute("**/api/edits");
    }
  });
  await step("Delete takes a wire drawn here out again", async () => {
    await clickWire([DIV, "AQ"], [METER, target]);
    const n = edits.length;
    await page.keyboard.press("Delete");
    await wireShown([DIV, "AQ"], [METER, target], false);
    assert.deepEqual(edits[n].ops, [{ op: "unwire", from: [DIV, "AQ"], to: [METER, target] }]);
    assert.doesNotMatch(wireFile(), /<-/);
    await page.waitForFunction(() => !/wire/.test(document.querySelector("#deploy-pend")?.textContent || ""));
  });
  await step("the undo history survives a reload: Delete is undone after it", async () => {
    const has = ([f, t, there]) => S.viewRev === S.rev && curPage()?.wires.some((w) => w.from.join() === f.join() && w.to.join() === t.join() && w.status !== "removed") === there;
    await page.reload();
    await page.waitForSelector(".tab");
    await page.waitForFunction(() => UNDO.blob && S.view && S.viewRev === S.rev, null, { timeout: 10000 });
    assert.match(await page.evaluate(() => UNDO.done.at(-1)?.label || ""), /^Delete wire /);
    await page.evaluate(([pg, u]) => { gotoPage(pg); focusObj(u, 0.05); }, [EG, DIV]);
    let ops = await undoKey(false, has, [[DIV, "AQ"], [METER, target], true]);
    assert.deepEqual(ops, [{ op: "wire", from: [DIV, "AQ"], to: [METER, target] }]);
    ops = await undoKey(true, has, [[DIV, "AQ"], [METER, target], false]);
    assert.deepEqual(ops, [{ op: "unwire", from: [DIV, "AQ"], to: [METER, target] }]);
  });
  await step("an older revision shows no managed blocks: the lockfile is today's", async () => {
    const pending = await page.evaluate(() => editRev().id);
    assert.match(pending, /^pending:/);
    const deployed = await page.evaluate(() => S.hist.history[0].id);
    await page.selectOption("#sel-rev", deployed);
    await page.waitForFunction((r) => S.viewRev === r, deployed, { timeout: 10000 });
    await page.evaluate((u) => gotoPage(S.objPage.get(u), { focus: u }), FLUR);
    await page.waitForSelector(`.blk[data-uuid="${FLUR}"]`);
    assert.equal(await page.$(".mbadge"), null);
    assert.equal(await page.$("#legend.managed"), null);
    // Back on the newest config, following it again.
    await page.selectOption("#sel-rev", pending);
    await page.evaluate(() => { const f = $("chk-follow"); f.checked = true; f.dispatchEvent(new Event("change")); });
    await page.waitForFunction((r) => S.follow && S.viewRev === r, pending, { timeout: 10000 });
    await page.evaluate((u) => gotoPage(S.objPage.get(u), { focus: u }), FLUR);
    await page.waitForSelector(`.blk.managed[data-uuid="${FLUR}"] .mbadge`);
    await page.waitForSelector("#legend.managed");
  });
  await step("a Discard refused while git is busy says so and changes nothing (#122)", async () => {
    const lock = join(dir, ".git/index.lock");
    const vislox = () => readdirSync(join(dir, "pages/vislox")).sort().map((f) => [f, readFileSync(join(dir, "pages/vislox", f), "utf8")]);
    const files = vislox(), out = readFileSync(join(dir, "out.Loxone")), index = readFileSync(join(dir, ".git/index"));
    assert.ok(files.length > 0);
    writeFileSync(lock, "");
    const seen = errors.length;
    try {
      await page.click("#btn-discard");
      // The reply and the actions event each show it, in either order: with or without the heading.
      await waitText("#deploy-msg", /^(Discard failed\.\n)?git is busy in this repository \(.*index\.lock is there\): try again when it is done\nNothing was discarded\.$/);
      assert.match(await page.getAttribute("#deploy-msg", "class"), /\berror\b/);
      // The reply itself has come (the browser logs its status).
      for (const until = Date.now() + 10000; !errors.slice(seen).some((e) => /status of 502/.test(e));) {
        assert.ok(Date.now() < until, "no reply to the refused Discard");
        await page.waitForTimeout(50);
      }
    } finally {
      rmSync(lock, { force: true });
    }
    // The refusal's own status in the console is expected; nothing else is.
    const now = errors.splice(seen);
    assert.ok(now.every((e) => /status of 502/.test(e)), now.join(" | "));
    assert.deepEqual(vislox(), files);
    assert.deepEqual(readFileSync(join(dir, "out.Loxone")), out);
    assert.deepEqual(readFileSync(join(dir, ".git/index")), index);
    await page.waitForFunction(() => !/Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""));
  });
  await step("Discard drops the edits", async () => {
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    assert.equal(existsSync(join(dir, "pages/vislox")), false);
    await page.waitForFunction(() => !document.querySelector("#props .ed-tag"));
  });
  await step("Discard clears the undo history and says why", async () => {
    await page.waitForFunction(() => /Discard dropped the pending config/.test(UNDO.note) && !UNDO.done.length, null, { timeout: 10000 });
    await page.waitForSelector("#btn-undo[disabled]");
    assert.match(await page.getAttribute("#btn-undo", "title"), /Nothing to undo\nUndo history cleared: Discard dropped/);
    await page.click("#btn-undo-list");
    await waitText("#undo-pop .u-note", /Discard dropped the pending config/);
    await page.click("#btn-undo-list");
    // The key sends nothing now.
    const n = edits.length;
    await page.keyboard.press("Control+z");
    await page.waitForTimeout(500);
    assert.equal(edits.length, n);
  });
  await step("a compile made outside this tab clears it too", async () => {
    await pick(DIV);
    await page.click('#props [data-ed="open"]');
    await page.fill(field("title"), "Außen");
    await page.press(field("title"), "Enter");
    await page.waitForFunction(() => UNDO.done.length === 1 && S.rev?.startsWith("pending:") && S.viewRev === S.rev, null, { timeout: 10000 });
    // Someone compiles the module elsewhere: the pending config is another one.
    writeFileSync(join(dir, "out.Loxone"), readFileSync(join(dir, "out.Loxone"), "utf8") + "\n<!-- compiled elsewhere -->\n");
    await page.waitForFunction(() => /changed outside this tab/.test(UNDO.note) && !UNDO.done.length, null, { timeout: 15000 });
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    assert.equal(existsSync(join(dir, "pages/vislox")), false);
  });
  await step("a block Loxone Config drew is adopted into the module, and the config stays as it is (#72)", async () => {
    const outWas = readFileSync(join(dir, "out.Loxone"), "utf8"), lockWas = readFileSync(join(dir, "logic.lock.json"), "utf8");
    await pick(DIV);
    await page.waitForSelector('#props [data-ed="adopt-open"]:not([disabled])');
    await page.click('#props [data-ed="adopt-open"]');
    // A slug that is not vislox's is refused in the browser.
    const n = edits.length;
    await page.fill("#ad-slug", "Teiler");
    await page.press("#ad-slug", "Enter");
    await waitText("#props", /A slug is vx_ and then lowercase letters/);
    assert.equal(edits.length, n);
    await page.fill("#ad-slug", "vx_teiler");
    await page.click('#props [data-ed="adopt"]');
    await page.waitForFunction((u) => S.edits?.blocks?.some((b) => b.uuid === u && b.adopted), DIV, { timeout: 20000 });
    await settled();
    assert.deepEqual(edits.at(-1).ops, [{ op: "adopt", blocks: [{ uuid: DIV, slug: "vx_teiler" }] }]);
    await waitText("#props", /Adopted as vx_teiler: the config is unchanged/);
    await waitText('[data-body="Edit"]', /Lox Logic\s*adopted here/);
    // Managed now, declared in vislox's file of its page; its wire's source is vislox's extern, by UUID.
    await page.waitForSelector(`.blk.managed[data-uuid="${DIV}"] .mbadge`);
    await waitText('[data-body="Lox Logic"]', /vx_teiler[\s\S]*pages\/vislox\/erdgeschoss\.loxl:\d+/);
    const file = readFileSync(join(dir, "pages/vislox/erdgeschoss.loxl"), "utf8");
    assert.match(file, /^# page uuid: 10000002-0000-0020-ffff504f94000001\npage "Erdgeschoss"$/m);
    assert.match(file, /^vx_teiler = Div\("O1002", In1: vx_10000001_0000_0010_ffff504f94000001\)$/m);
    assert.match(readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8"), /^extern vx_10000001_0000_0010_ffff504f94000001 = VirtualIn\(uuid: "10000001-0000-0010-ffff504f94000001"\)$/m);
    assert.equal(JSON.parse(readFileSync(join(dir, "logic.lock.json"), "utf8")).objects.vx_teiler.uuid, DIV);
    // Nothing the Miniserver runs changed: nothing pending, and the adoption goes into git with the next Apply.
    assert.equal(readFileSync(join(dir, "out.Loxone"), "utf8"), outWas);
    await waitText("#deploy-pend", /1 block adopted here goes into git with the next Apply/);
    // Its name is edited in its declaration, like a block added here; undone, the declaration has it back.
    await page.click('#props [data-ed="open"]');
    assert.deepEqual(await page.$$eval("#props [data-ed-field]", (x) => x.map((f) => f.dataset.edField)), ["title", "p:Input2"]);
    await page.fill(field("title"), "Teiler");
    await page.press(field("title"), "Enter");
    await page.waitForFunction(() => S.rev?.startsWith("pending:") && S.viewRev === S.rev, null, { timeout: 15000 });
    assert.match(readFileSync(join(dir, "pages/vislox/erdgeschoss.loxl"), "utf8"), /^vx_teiler = Div\("Teiler", In1: /m);
    await settled();
    await undoKey(false, () => !S.rev?.startsWith("pending:"));
    assert.match(readFileSync(join(dir, "pages/vislox/erdgeschoss.loxl"), "utf8"), /^vx_teiler = Div\("O1002", In1: /m);
    // Undo hands it back to Loxone Config: vislox's file and the lock as they were.
    const ops = await undoKey(false, (u) => !S.edits?.blocks?.some((b) => b.uuid === u), DIV);
    assert.deepEqual(ops, [{ op: "unadopt", uuids: [DIV] }]);
    assert.equal(existsSync(join(dir, "pages/vislox/erdgeschoss.loxl")), false);
    assert.deepEqual(JSON.parse(readFileSync(join(dir, "logic.lock.json"), "utf8")).objects, JSON.parse(lockWas).objects);
    assert.equal(readFileSync(join(dir, "out.Loxone"), "utf8"), outWas);
    await page.waitForSelector('#props [data-ed="adopt-open"]');
  });
  await step("Adopt all takes what lox rebuilds on the page and says what stays Loxone Config's (#72)", async () => {
    const lockWas = readFileSync(join(dir, "logic.lock.json"), "utf8");
    await page.waitForSelector("#btn-adopt-page:not([disabled])");
    const n = edits.length;
    // While an undo compiles it is not sent, and the Deploy pane says why.
    await page.evaluate(() => { UNDO.busy++; try { adoptPage(); } finally { UNDO.busy--; } });
    await waitText("#deploy-msg", /^Cannot adopt now: An edit is compiling\.$/);
    assert.equal(edits.length, n);
    await page.click("#btn-adopt-page");
    // lox adopts each block, then the module compiles (slowly with VISLOX_TEST_LOX_DELAY).
    await until(() => statuses[n] !== undefined, "the adopt_page request");
    assert.equal(statuses[n], 200);
    assert.deepEqual(edits[n].ops, [{ op: "adopt_page", page: EG }]);
    await settled();
    await waitText("#deploy-msg", /Adopted 3 blocks on Erdgeschoss: the config is unchanged\. 1 block stays Loxone Config's: Temperatur \(cannot adopt InputRef/);
    await settled();
    const objs = JSON.parse(readFileSync(join(dir, "logic.lock.json"), "utf8")).objects;
    assert.deepEqual([objs.vx_o1001?.uuid, objs.vx_zaehler?.uuid, objs.vx_o1002?.uuid], [AND, METER, DIV]);
    await page.waitForSelector(`.blk.managed[data-uuid="${METER}"] .mbadge`);
    // A wire into an adopted block: its declaration draws it now, and vislox does not take it out.
    await page.waitForFunction(([f, t]) => S.edits?.adopted_wires?.some((w) => w.from.join() === f.join() && w.to.join() === t.join()),
      [[AND, "Q"], [METER, "Pf"]], { timeout: 20000 });
    await clickWire([AND, "Q"], [METER, "Pf"]);
    await waitText("#wcard", /brought along when the block was adopted/);
    assert.equal(await page.locator('#wcard [data-wc="unwire"]').count(), 0);
    await page.click('#wcard [data-wc="close"]');
    // Only the input reference is still offered: lox says why not when it is asked.
    await page.waitForFunction(() => adoptOnPage(addState().page).map((o) => o.title).join() === "Temperatur");
    // One undo hands them all back.
    const ops = await undoKey(false, () => !S.edits?.blocks?.length);
    assert.deepEqual(ops, [{ op: "unadopt", uuids: [AND, METER, DIV] }]);
    assert.deepEqual(JSON.parse(readFileSync(join(dir, "logic.lock.json"), "utf8")).objects, JSON.parse(lockWas).objects);
    assert.equal(existsSync(join(dir, "pages/vislox/erdgeschoss.loxl")), false);
  });
  await step("a module that does not compile is listed, error by error, and Apply is off (#65)", async () => {
    const flur = join(dir, "pages", "flur.loxl"), was = readFileSync(flur, "utf8");
    writeFileSync(flur, was + "flur.In1 <- BROKEN.Q\n");
    await waitText("#deploy-check", /The module doesn't compile: 1 error/);
    assert.match(await page.textContent("#deploy-check li"), /pages\/flur\.loxl:4unknown block type BROKEN/);
    assert.equal(await page.evaluate(() => document.querySelector("#btn-apply").disabled), true);
    // The error's line names `flur`: its button shows the block, and Properties says why.
    await page.click('#deploy-check .go:has-text("Flur")');
    await page.waitForFunction((u) => S.sel === u, FLUR);
    await waitText("#props", /Doesn't compile \(pages\/flur\.loxl:4\): unknown block type BROKEN/);
    // Fixed, but not compiled: the module changed since out, and Compile compiles it.
    writeFileSync(flur, was.replace('And("Flur")', 'And("Flur oben")'));
    await waitText("#deploy-check", /The module changed \(pages\/flur\.loxl\) and is not compiled yet/);
    await page.click("#btn-compile");
    await waitText("#deploy-pend", /Compiled changes are pending/);
    await page.waitForSelector("#deploy-check.hidden", { state: "attached" });
    assert.match(readFileSync(join(dir, "out.Loxone"), "utf8"), /Title="Flur oben"/);
    assert.equal(await page.evaluate(() => document.querySelector("#btn-apply").disabled), false);
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    assert.equal(readFileSync(flur, "utf8"), was);
  });
  await step("lint findings are marked on their blocks, listed, and on the Code Touch (#66)", async () => {
    const flur = join(dir, "pages", "flur.loxl"), was = readFileSync(flur, "utf8");
    writeFileSync(flur, was.replace('flur = And("Flur")', 'flur = And("Flur") # dead') + "# NOTAG\n");
    await waitText("#deploy-lint", /Lint: 2 findings/);
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); }, FLUR);
    await page.waitForSelector(`.blk.linted[data-uuid="${FLUR}"] .lbadge`);
    assert.match(await page.textContent(`.blk[data-uuid="${FLUR}"] .lbadge title`), /Lint \(dead-outputs\): no wire draws/);
    assert.equal(await page.$(`.blk[data-uuid="${MONO}"] .lbadge`), null);
    // The list, by kind: each finding with its file and line, and a button that shows its block.
    await page.click("#deploy-lint summary");
    assert.match(await page.textContent("#deploy-lint"), /dead-outputs \(1\)[\s\S]*pages\/flur\.loxl:3flur: no wire draws/);
    assert.match(await page.textContent("#deploy-lint"), /no-access-right \(1\)[\s\S]*Anna: user with an NFC tag/);
    await page.click('#deploy-lint .go:has-text("Flur")');
    await page.waitForFunction((u) => S.sel === u, FLUR);
    await waitText('[data-body="Lox Logic"]', /Lint \(dead-outputs, pages\/flur\.loxl:3\): no wire draws/);
    // A hint only: Apply is not turned off for it.
    assert.doesNotMatch(await page.getAttribute("#btn-apply", "title"), /lint/i);
    // The Code Touch says whose tag opens nothing.
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, TOUCH);
    await waitText('[data-body="Lox Logic"]', /Lint \(no-access-right\): Anna, user with an NFC tag has no access right/);
    // Fixed: the badge and the list go.
    writeFileSync(flur, was);
    await page.waitForSelector("#deploy-lint.hidden", { state: "attached" });
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); }, FLUR);
    await page.waitForSelector(`.blk[data-uuid="${FLUR}"]`);
    assert.equal(await page.$(`.blk[data-uuid="${FLUR}"] .lbadge`), null);
    await page.waitForSelector("#deploy-check.hidden", { state: "attached" });
  });
  await step("a failing test is listed with expected and actual, marked on its block, and Apply is off (#67)", async () => {
    const flur = join(dir, "pages", "flur.loxl"), was = readFileSync(flur, "utf8");
    // A test of `flur`, and a rename, so that Compile makes something pending.
    const withTest = (mark) => was.replace('And("Flur")', 'And("Flur getestet")') +
      `\ntest "Flur schaltet"\n\tflur.In1 = 1\n\tflur.In2 = 1\n\ttick 1s\n\texpect flur.Q == 1 # ${mark}\nend\n`;
    writeFileSync(flur, withTest("FAILTEST"));
    await waitText("#deploy-check", /The module changed \(pages\/flur\.loxl\) and is not compiled yet/);
    // The check's results come before the pending revision's view, or the view never comes (#126): the links
    // name the blocks of the revision shown, so with another one shown (or loading) there are none, and with the
    // pending one loaded they take their names from it, not from the view it replaced.
    // What the box shows the moment each load of a view is done: no later event may be what corrects it.
    await page.evaluate(() => {
      window.__loaded = [];
      window.__loadView = loadView;
      loadView = async function (...a) {
        await window.__loadView.apply(this, a);
        window.__loaded.push({ rev: S.viewRev, go: [...document.querySelectorAll("#deploy-tests .go")].map((b) => b.textContent) });
      };
    });
    const finalState = async () => {
      await page.waitForFunction(() => S.check?.current && S.check.test?.available && !S.check.test.running && S.check.test.failed === 1, null, { timeout: 15000 });
      await waitText("#deploy-tests", /Tests: 0 of 1 passed, 1 failed/);
      // The view of the revision before the compile is the one shown: the pending one is not.
      assert.notEqual(await page.evaluate(() => S.viewRev), await page.evaluate(() => S.rev));
      assert.deepEqual(await goLabels("#deploy-tests"), []);
      assert.doesNotMatch(await page.textContent("#deploy-tests"), /Flur(?! schaltet)/);
    };
    // A view that fails to load: no buttons with the names of the view before.
    let route = await viewRoute("fail");
    const seen = errors.length;
    try {
      await page.click("#btn-compile");
      await waitText("#deploy-pend", /Compiled changes are pending/);
      await finalState();
      await until(() => route.failed > 0, "a refused view");
      await finalState();
    } finally { await route.stop(); }
    // The refused views' own status in the console is expected; nothing else is.
    const refused = errors.splice(seen);
    assert.ok(refused.length > 0 && refused.every((e) => /status of 500/.test(e)), refused.join(" | "));
    // A view that comes after the check's answer: held until it is in.
    route = await viewRoute("hold");
    try {
      await page.evaluate(() => { window.__again = loadView(); });
      await finalState();
    } finally { await route.stop(); }
    await page.waitForFunction(() => S.viewRev === S.rev && S.rev.startsWith("pending:") && window.__loaded.some((l) => l.rev === S.rev), null, { timeout: 15000 });
    const loaded = await page.evaluate(() => { loadView = window.__loadView; return window.__loaded.filter((l) => l.rev.startsWith("pending:")); });
    // Every link names its block as the loaded revision does ("Flur getestet"), as soon as the view is in.
    assert.ok(loaded.length > 0 && loaded.every((l) => l.go.length > 0 && l.go.every((t) => t === "Flur getestet")), JSON.stringify(loaded));
    // Open while it fails: the test with its file and line, the expect with expected and actual, and its block.
    assert.match(await page.textContent("#deploy-tests"), /pages\/flur\.loxl:5Flur schaltet[\s\S]*pages\/flur\.loxl:9step 1: expect flur\.Q == 1 — expected 1, actual 0/);
    assert.match(await page.textContent("#deploy-tests"), /A pass covers only what the simulator models/);
    assert.equal(await page.evaluate(() => document.querySelector("#btn-apply").disabled), true);
    assert.match(await page.getAttribute("#btn-apply", "title"), /Cannot apply: 1 test of lox logic test fails/);
    await page.click('#deploy-tests .go:has-text("Flur getestet")');
    await page.waitForFunction((u) => S.sel === u, FLUR);
    await page.waitForSelector(`.blk.test-failed[data-uuid="${FLUR}"] .tbadge`);
    assert.match(await page.textContent(`.blk[data-uuid="${FLUR}"] .tbadge title`), /Test "Flur schaltet" \(pages\/flur\.loxl:5\) fails: step 1: expect flur\.Q == 1/);
    await waitText('[data-body="Lox Logic"]', /✕ Test "Flur schaltet" \(pages\/flur\.loxl:5\) fails/);
    // Fixed: it passes, the badge goes, and Apply is on again (the module changed since the compile, which
    // Apply only warns about).
    writeFileSync(flur, withTest("fixed"));
    await page.waitForFunction(() => S.check?.current && S.check.test?.available && !S.check.test.running && S.check.test.passed === 1, null, { timeout: 15000 });
    await waitText("#deploy-tests", /Tests: 1 of 1 passed/);
    await page.waitForFunction((u) => !document.querySelector(`.blk[data-uuid="${u}"] .tbadge`), FLUR);
    assert.equal(await page.evaluate(() => document.querySelector("#btn-apply").disabled), false);
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    assert.equal(readFileSync(flur, "utf8"), was);
    // No tests: the box goes.
    await page.waitForSelector("#deploy-tests.hidden", { state: "attached", timeout: 15000 });
  });
  await step("a block the module lost: the Deploy pane says so, and its removal is reviewed before Apply (#70)", async () => {
    const flur = join(dir, "pages", "flur.loxl"), was = readFileSync(flur, "utf8");
    const lockWas = readFileSync(join(dir, "logic.lock.json"), "utf8"), outWas = readFileSync(join(dir, "out.Loxone"), "utf8");
    writeFileSync(flur, was.replace('flur = And("Flur")\n', ""));
    await waitText("#deploy-check", /The module no longer has managed blocks: lox deletes them only when you allow it/);
    assert.match(await page.textContent("#deploy-check"), /flur is in the lockfile but not in the source/);
    // Apply is on: it asks for exactly these removals first.
    assert.equal(await page.evaluate(() => document.querySelector("#btn-apply").disabled), false);
    await page.click("#btn-removals");
    // What Apply would deploy, compared with what runs, and the block: where it is and what is wired to it.
    await page.waitForSelector(".hrev.k-removals.sel");
    await waitText("#deploy-check", /Flur \(And, flur\) on page "Obergeschoss": nothing wired/);
    await page.waitForFunction(() => S.viewRev === S.rev && S.view?.compared);
    assert.match(await page.textContent(".changes"), /Flur/);
    // Its button shows the block, marked as removed in the comparison.
    await page.click('#deploy-check .rm-list .go:has-text("Flur")');
    await page.waitForFunction((u) => S.sel === u && S.view.objects[u]?.status === "removed", FLUR);
    // Cancelled from another tab: the server forgets the plan, and this tab's preview goes with it.
    const other = await browser.newPage();
    await other.goto(page.url());
    const rev = await page.evaluate(() => S.removals.rev);
    await other.evaluate((r) => fetch("/api/removals/cancel", { method: "POST", headers: { "Content-Type": "application/json", "X-Vislox": "removals" }, body: JSON.stringify({ rev: r }) }), rev);
    await other.close();
    await page.waitForFunction(() => !S.removals && !document.querySelector(".hrev.k-removals"), null, { timeout: 10000 });
    await waitText("#deploy-check", /Review the removals/);
    // The review changed nothing in the project.
    assert.equal(readFileSync(join(dir, "logic.lock.json"), "utf8"), lockWas);
    assert.equal(readFileSync(join(dir, "out.Loxone"), "utf8"), outWas);
    // Declared again: nothing to remove.
    writeFileSync(flur, was);
    await page.waitForSelector("#deploy-check.hidden", { state: "attached", timeout: 15000 });
    await page.click(".hrev.k-git >> nth=0");
  });
  await step("a Daytimer's switching times are edited as a schedule set, undone, shared and reverted (#73)", async () => {
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    await page.evaluate(() => editGoNewest());
    await settled();
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, UHR);
    await page.waitForSelector('#props [data-body="Switching times"] [data-se="open"]');
    await page.click('#props [data-se="open"]');
    // A row per weekday, then the timer's Party column; Monday is selected, with its interval.
    const modes = await page.$$eval("#props .se-row .se-mode", (n) => n.map((x) => x.textContent));
    assert.deepEqual(modes, ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday", "Party"]);
    const sel = "#props .se-detail";
    assert.deepEqual(await page.$$eval(`${sel} input.se-t`, (n) => n.map((x) => x.value)), ["06:30", "08:00"]);
    // Typed: until 09:00. Then a click on the empty bar at 18:00 adds an hour there, dragged out to 21:00.
    await page.fill(`${sel} [data-sch-field="${MON}|0|to"]`, "09:00");
    const bar = await page.$(`#props [data-se-bar="${MON}"]`);
    const box = await bar.boundingBox();
    const x = (min) => box.x + (box.width * min) / 1440;
    await page.mouse.move(x(1080), box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(x(1200), box.y + box.height / 2, { steps: 4 });
    await page.mouse.move(x(1260), box.y + box.height / 2, { steps: 4 });
    await page.mouse.up();
    await page.waitForFunction((s) => document.querySelectorAll(`${s} .se-iv`).length === 2, sel);
    assert.deepEqual(await page.$$eval(`${sel} input.se-t`, (n) => n.map((x) => x.value)), ["06:30", "09:00", "18:00", "21:00"]);
    await page.fill(`${sel} [data-sch-field="${MON}|1|value"]`, "22.5");
    await page.check(`${sel} [data-se="act"][data-i="1"]`);
    await page.click(`${sel} [data-se="copy"][data-to="weekdays"]`);
    // Saturday: one past midnight is refused in the browser, and nothing is sent.
    await page.click(`#props .se-mode:has-text("Saturday")`);
    await page.click(`${sel} [data-se="add"]`);
    await page.fill(`${sel} [data-sch-field="${dayU(9)}|0|from"]`, "22:00");
    await page.fill(`${sel} [data-sch-field="${dayU(9)}|0|to"]`, "02:00");
    const n = edits.length;
    await page.click('#props [data-se="save"]');
    await waitText('[data-body="Switching times"]', /Fix the marked intervals/);
    assert.match(await page.textContent(sel), /split an interval across midnight into two/);
    assert.equal(edits.length, n);
    await page.click(`${sel} [data-se="del"][data-i="0"]`);
    await page.click('#props [data-se="save"]');
    await until(() => edits.length > n && statuses[n] !== undefined, "the schedule request");
    assert.equal(statuses[n], 200);
    const [op] = edits[n].ops;
    assert.equal(op.op, "schedule");
    assert.equal(op.uuid, UHR);
    const wk = [{ from: 390, to: 540, value: "21" }, { from: 1080, to: 1260, value: "22.5", activate: true }];
    for (const d of [4, 5, 6, 7, 8]) assert.deepEqual(op.value.modes[dayU(d)], wk);
    assert.deepEqual(op.value.modes[dayU(9)], []);
    assert.deepEqual(op.value.modes[PARTY.toLowerCase()], [{ from: 1080, to: 1380, value: "22.5" }]);
    await settled();
    await page.waitForFunction((u) => S.edits?.schedules?.some((s) => s.uuid === u && s.pending), UHR, { timeout: 10000 });
    const file = readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    assert.match(file, /^schedule vx_uhr_schedule\n\tweekdays 06:30-09:00 = 21, 18:00-21:00 = 22\.5 activate\n\tweekend off\n\t"Party" 18:00-23:00 = 22\.5\nend$/m);
    assert.match(file, /^vx_10000020_0000_0200_ffff504f94000001\.schedule = vx_uhr_schedule$/m);
    // Compiled: the pending config has the new switching times, marked not deployed, and the Deploy pane counts them.
    await waitText('[data-body="Switching times"]', /edited here[\s\S]*not deployed[\s\S]*Revert/);
    assert.match(await page.textContent('[data-body="Switching times"]'), /Written by vislox as vx_uhr_schedule/);
    await page.waitForFunction((u) => S.view.objects[u].schedule.entries.filter((e) => e.from === 1080 && e.to === 1260 && e.activate).length === 5, UHR);
    await waitText("#deploy-pend", /switching times of 1 timer edited here/);
    // Undone and redone through the reply's undo.
    let ops = await undoKey(false, (u) => S.view.objects[u].schedule.entries.length === 2, UHR);
    assert.deepEqual(ops, [{ op: "schedule", uuid: UHR, value: null }]);
    assert.doesNotMatch(readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8"), /schedule/);
    ops = await undoKey(true, (u) => S.view.objects[u].schedule.entries.length === 11, UHR);
    assert.equal(ops[0].op, "schedule");
    assert.deepEqual(ops[0].value.modes[dayU(4)], wk);
    // Heizung takes the same times: one set, two claims.
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, HEIZ);
    await page.waitForSelector('#props [data-se="same"]');
    const m = edits.length;
    await page.selectOption('#props [data-se="same"]', "vx_uhr_schedule");
    await until(() => edits.length > m && statuses[m] !== undefined, "the shared schedule request");
    assert.equal(statuses[m], 200);
    assert.deepEqual(edits[m].ops, [{ op: "schedule", uuid: HEIZ, value: { set: "vx_uhr_schedule" } }]);
    await settled();
    await page.waitForFunction((u) => S.view.objects[u].schedule.entries.length === 11, HEIZ);
    assert.match(readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8"), /^vx_10000021_0000_0210_ffff504f94000001\.schedule = vx_uhr_schedule$/m);
    await waitText('[data-body="Switching times"]', /shared with Uhr/);
    // A digital timer is not offered the analog set: the server would refuse it.
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, LICHT);
    await page.waitForSelector('#props [data-body="Switching times"] [data-se="open"]');
    assert.equal(await page.$('#props [data-se="same"]'), null);
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, HEIZ);
    await page.waitForSelector('#props [data-se="revert"]');
    // Reverted: back to Loxone Config's (no claim); the set stays for Uhr.
    await page.click('#props [data-se="revert"]');
    await page.waitForFunction((u) => S.view.objects[u].schedule.entries.length === 0 && !S.edits?.schedules?.some((s) => s.uuid === u), HEIZ, { timeout: 15000 });
    await settled();
    assert.doesNotMatch(readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8"), /0210_ffff504f94000001\.schedule/);
    // Claimed twice: the module's own file claims Uhr too. vislox's claim is removed with one click, and the
    // module's file keeps the timer.
    const own = join(dir, "pages", "uhr.loxl");
    writeFileSync(own, `# The module's own.\nextern uhr = DayTimer(uuid: "${UHR}")\nschedule abend\n\tmon 17:00-22:00 = 20\nend\nuhr.schedule = abend\n`);
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, UHR);
    await waitText('[data-body="Switching times"]', /Claimed twice: pages\/uhr\.loxl:6 and vislox's/);
    const k = edits.length;
    await page.click('#props [data-se="drop"]');
    await until(() => edits.length > k && statuses[k] !== undefined, "the drop request");
    assert.equal(statuses[k], 200);
    assert.deepEqual(edits[k].ops, [{ op: "schedule", uuid: UHR, value: null }]);
    await settled();
    assert.doesNotMatch(readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8"), /\.schedule = /);
    await waitText('[data-body="Switching times"]', /pages\/uhr\.loxl:6 claims its switching times: change them there/);
    rmSync(own);
    // Both timers are as deployed again: nothing pending.
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
  });
  await step("a new interval never overlaps, and Revert changes no other timer (#73)", async () => {
    // seFree: an hour from the start, no later than 23:00; none inside an interval or without a free minute.
    const free = (spans, at) => page.evaluate(([sp, a]) => seFree({ spans: sp }, a), [spans, at]);
    assert.equal(await free([{ from: 1350, to: 1420 }], 1435), null);
    assert.deepEqual(await free([{ from: 1410, to: 1440 }], 1435), { from: 1380, to: 1410 });
    assert.deepEqual(await free([{ from: 400, to: 500 }], 360), { from: 360, to: 400 });
    assert.equal(await free([{ from: 0, to: 1440 }], 600), null);
    // seAdd finds another free hour when the clicked one is taken, and says so when the day is full.
    assert.equal(await page.evaluate(() => { const c = { spans: [{ from: 0, to: 1440 }] }; return seAdd(c, 600, false); }), null);
    assert.deepEqual(await page.evaluate(() => { const c = { spans: [{ from: 1350, to: 1440 }] }; seAdd(c, 1435, false); return c.spans.map((s) => [s.from, s.to]); }), [[0, 60], [1350, 1440]]);
    // setRevertValue: the deployed set while only this timer claims it; else the deployed times in a set of its
    // own, never a rewrite of the others'; refused when the deployed times cannot be read.
    const M = { [dayU(4)]: [{ from: 60, to: 120, value: "20" }] };
    const rv = (e, sets) => page.evaluate(([e, sets, u]) => setRevertValue("schedule", e, sets, u), [e, sets, UHR]);
    assert.deepEqual(await rv({ deployed: null }, []), { value: null });
    assert.deepEqual(await rv({ deployed: { set: "vx_a", modes: M } }, [{ name: "vx_a", timers: [UHR] }]), { value: { set: "vx_a", modes: M } });
    assert.deepEqual(await rv({ deployed: { set: "vx_a", modes: M } }, [{ name: "vx_a", timers: [UHR, HEIZ] }]), { value: { modes: M } });
    assert.deepEqual(await rv({ deployed: { set: "vx_a", modes: M } }, [{ name: "vx_a", timers: [HEIZ] }]), { value: { modes: M } });
    assert.match((await rv({ deployed: { set: "vx_a", modes: null } }, [{ name: "vx_a", timers: [HEIZ] }])).why, /Heizung claims that set now/);
    // Unreadable and nobody else on it: refused too, never the set's name alone (which would change nothing).
    assert.match((await rv({ deployed: { set: "vx_a", modes: null } }, [{ name: "vx_a", timers: [UHR] }])).why, /deployed switching times \(vx_a\) cannot be read/);
  });
  await step("a lighting controller's moods are shown, edited as a moods set, undone, shared and reverted (#74)", async () => {
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    await page.evaluate(() => editGoNewest());
    await settled();
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, LC_W);
    const sec = '#props [data-body="Moods"]';
    await page.waitForSelector(`${sec} [data-mo="open"]`);
    // The circuits in use, each with its kind; the moods in order, the off mood marked, Lesen selected by T5/1.
    const body = await page.textContent(sec);
    assert.match(body, /AQ1\s*Deckenlicht switch/);
    assert.match(body, /AQ2\s*no actuator dimmer/);
    assert.match(body, /AQ3\s*no actuator tunable white/);
    assert.match(body, /Lesen\s*AQ1 on, AQ2 80%\s*T5\/1 selects it/);
    assert.match(body, /Aus\s*every circuit off, always there\s*off mood/);
    assert.match(body, /Viel Licht\s*all on, AQ2 100%, AQ3 temp\(100%, 2700K\)\s*built-in/);
    await page.click(`${sec} [data-mo="open"]`);
    assert.deepEqual(await page.$$eval("[data-mo-name]", (n) => n.map((x) => x.value)), ["Viel Licht", "Lesen", "Kino"]);
    // "Viel Licht" is built in: no Delete, and its name stays.
    assert.equal(await page.$eval('[data-mo="del"][data-i="0"]', (b) => b.disabled), true);
    assert.equal(await page.$eval('[data-mo-name="0"]', (b) => b.readOnly), true);
    // Only the values a circuit takes: a dimmer has no "on", tunable white no "%".
    assert.deepEqual(await page.$$eval('[data-mo="t"][data-k="1|2"] option', (n) => n.map((x) => x.value)), ["off", "pct"]);
    assert.deepEqual(await page.$$eval('[data-mo="t"][data-k="1|3"] option', (n) => n.map((x) => x.value)), ["off", "temp", "rgb"]);
    // Lesen dims to 40% (first a value the dimmer cannot take: refused in the browser, nothing sent).
    await page.fill('[data-mo-field="1|2"]', "140");
    const n0 = edits.length;
    await page.click(`${sec} [data-mo="save"]`);
    await waitText(sec, /Fix the marked moods and values/);
    assert.equal(edits.length, n0);
    await page.fill('[data-mo-field="1|2"]', "40");
    // Kino becomes Film; a new mood, Abend, in tunable white, moved before Film.
    await page.fill('[data-mo-name="2"]', "Film");
    await page.click(`${sec} [data-mo="add"]`);
    await page.fill('[data-mo-name="3"]', "Abend");
    await page.selectOption('[data-mo="t"][data-k="3|3"]', "temp");
    await page.fill('[data-mo-field="3|3"]', "30, 2700");
    await page.click('[data-mo="up"][data-i="3"]');
    // Renaming Lesen, which a button selects, is warned about first; named back, it saves at once.
    await page.fill('[data-mo-name="1"]', "Lesen 2");
    await page.click(`${sec} [data-mo="save"]`);
    await waitText(sec, /T5\/1 selects "Lesen", which this renames to "Lesen 2"/);
    assert.equal(await page.textContent(`${sec} [data-mo="save"]`), "Save anyway");
    assert.equal(edits.length, n0);
    await page.fill('[data-mo-name="1"]', "Lesen");
    await page.click(`${sec} [data-mo="save"]`);
    await until(() => edits.length > n0 && statuses[n0] !== undefined, "the moods request");
    assert.equal(statuses[n0], 200);
    const [op] = edits[n0].ops;
    assert.equal(op.op, "moods");
    assert.equal(op.uuid, LC_W);
    assert.deepEqual(op.value.moods.map((m) => m.name), ["Viel Licht", "Lesen", "Abend", "Film"]);
    assert.deepEqual(op.value.moods.slice(1).map((m) => m.values), ["AQ1 on, AQ2 40%", "AQ3 temp(30%, 2700K)", "AQ3 temp(20%, 2700K)"]);
    await settled();
    const file = readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    assert.match(file, /^moods vx_wohnzimmer_moods\n\t"Viel Licht": AQ1 on, AQ2 100%, AQ3 temp\(100%, 2700K\), AQ4 on, [^\n]*\n\t"Lesen": AQ1 on, AQ2 40%\n\t"Abend": AQ3 temp\(30%, 2700K\)\n\t"Film": AQ3 temp\(20%, 2700K\)\nend$/m);
    assert.match(file, /^vx_10000040_0000_0400_ffff504f94000001\.moods = vx_wohnzimmer_moods$/m);
    // Compiled: the pending config has the new moods (Film a new mood: Kino's scene id went to the first new
    // mood, Abend), marked not deployed; the change list says it in the language's terms.
    await page.waitForFunction((u) => S.view.objects[u].moods.moods.map((m) => `${m.name}:${m.sid}`).join() === "Viel Licht:777,Lesen:1,Abend:2,Film:3,Aus:778", LC_W, { timeout: 10000 });
    await waitText(sec, /edited here[\s\S]*not deployed[\s\S]*Revert/);
    assert.match(await page.textContent(sec), /Written by vislox as vx_wohnzimmer_moods/);
    await waitText("#deploy-pend", /moods of 1 lighting controller edited here/);
    const changes = await page.evaluate(() => S.view.changes.map((c) => c.text));
    assert.ok(changes.includes('Wohnzimmer · mood "Lesen": AQ1 on, AQ2 80% → AQ1 on, AQ2 40%'), changes.join(" | "));
    assert.ok(changes.includes('Wohnzimmer · mood "Kino" removed (was AQ3 temp(20%, 2700K))'), changes.join(" | "));
    assert.ok(changes.includes('Wohnzimmer · mood "Film" added: AQ3 temp(20%, 2700K)'), changes.join(" | "));
    // lox logic lint: a percentage on AQ2, which has no actuator, only switches it; said on the circuit.
    await waitText(sec, /AQ2[\s\S]*⚠ Lint \(no-actuator[\s\S]*AQ2 has no actuator/);
    // Undone and redone through the reply's undo.
    let ops = await undoKey(false, (u) => S.view.objects[u].moods.moods.map((m) => m.name).join() === "Viel Licht,Lesen,Kino,Aus", LC_W);
    assert.deepEqual(ops, [{ op: "moods", uuid: LC_W, value: null }]);
    assert.doesNotMatch(readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8"), /moods/);
    ops = await undoKey(true, (u) => S.view.objects[u].moods.moods.length === 5, LC_W);
    assert.equal(ops[0].op, "moods");
    assert.equal(ops[0].value.moods[2].name, "Abend");
    // Diele takes the same moods: one set, two claims.
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, LC_F);
    await page.waitForSelector('#props [data-mo="same"]');
    const m = edits.length;
    await page.selectOption('#props [data-mo="same"]', "vx_wohnzimmer_moods");
    await until(() => edits.length > m && statuses[m] !== undefined, "the shared moods request");
    assert.equal(statuses[m], 200);
    assert.deepEqual(edits[m].ops, [{ op: "moods", uuid: LC_F, value: { set: "vx_wohnzimmer_moods" } }]);
    await settled();
    await page.waitForFunction((u) => S.view.objects[u].moods.moods.length === 5, LC_F);
    await waitText(sec, /shared with Wohnzimmer/);
    // Reverted: Diele's claim goes; Wohnzimmer keeps the set.
    await page.click(`${sec} [data-mo="revert"]`);
    await page.waitForFunction((u) => S.view.objects[u].moods.moods.length === 4 && !S.edits?.moods?.some((x) => x.uuid === u), LC_F, { timeout: 15000 });
    await settled();
    assert.match(readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8"), /^vx_10000040_0000_0400_ffff504f94000001\.moods = vx_wohnzimmer_moods$/m);
    assert.doesNotMatch(readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8"), /0410_ffff504f94000001\.moods/);
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, LC_W);
    await page.waitForSelector(`${sec} [data-mo="revert"]`);
    await page.click(`${sec} [data-mo="revert"]`);
    await page.waitForFunction((u) => S.view.objects[u].moods.moods.length === 4 && !S.edits?.moods?.length, LC_W, { timeout: 15000 });
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    // setRevertValue: the deployed set while only this controller claims it; else the deployed moods in a set of
    // its own, never a rewrite of the others'.
    const M = [{ name: "Viel Licht", values: "all on" }];
    const rv = (e, sets) => page.evaluate(([e, sets, u]) => setRevertValue("moods", e, sets, u), [e, sets, LC_W]);
    assert.deepEqual(await rv({ deployed: null }, []), { value: null });
    assert.deepEqual(await rv({ deployed: { set: "vx_a", moods: M } }, [{ name: "vx_a", controllers: [LC_W] }]), { value: { set: "vx_a", moods: M } });
    assert.deepEqual(await rv({ deployed: { set: "vx_a", moods: M } }, [{ name: "vx_a", controllers: [LC_W, LC_F] }]), { value: { moods: M } });
    assert.match((await rv({ deployed: { set: "vx_a", moods: null } }, [{ name: "vx_a", controllers: [LC_F] }])).why, /Diele claims that set now/);
    assert.match((await rv({ deployed: { set: "vx_a", moods: null } }, [])).why, /deployed moods \(vx_a\) cannot be read/);
    // A claimant the config lost is named, with Drop its claim (the server refuses the set until it goes).
    const stale = await page.evaluate(() => staleRows({ name: "vx_a", stale: ["10000099-0000-0990-ffff504f94000001"] }, "mo", null).join(""));
    assert.match(stale, /vx_a is also claimed for 10000099-0000-0990-ffff504f94000001, which this config does not have/);
    assert.match(stale, /data-mo="dropstale" data-u="10000099-0000-0990-ffff504f94000001"[^>]*>Drop its claim/);
  });
  await step("a central block's members are shown, edited in order with the Ctrl+K picker, emptied, undone and reverted (#75)", async () => {
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    await page.evaluate(() => editGoNewest());
    await settled();
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, CENTRAL);
    const sec = '#props [data-body="Members"]';
    await page.waitForSelector(`${sec} [data-me="open"]`);
    // The member, a link to its block.
    assert.match(await page.textContent(sec), /1\s*Wohnzimmer/);
    assert.equal(await page.getAttribute(`${sec} .lnk`, "data-go"), LC_W);
    // With the editor closed, Ctrl+K is the usual palette.
    await page.keyboard.press("Control+k");
    await page.waitForSelector("#palette:not(.hidden)");
    assert.equal(await page.getAttribute("#pal-input", "placeholder"), "Go to a page or block…");
    await page.keyboard.press("Escape");
    await page.waitForSelector("#palette.hidden", { state: "attached" });
    // A member the config lacks is left out of the draft, and the editor says so.
    await page.evaluate((u) => { S.view.objects[u].members.members.push({ uuid: "10000099-0000-0990-ffff504f94000001", missing: true }); meOpen(u); }, CENTRAL);
    assert.deepEqual(await page.evaluate(() => ME.list), [LC_W]);
    await waitText(sec, /1 member not in this config was dropped from the draft/);
    await page.evaluate((u) => { S.view.objects[u].members.members.pop(); meClose(); }, CENTRAL);
    // The picker never offers a block the module manages (the server's `managed`).
    await page.click(`${sec} [data-me="open"]`);
    await page.evaluate((u) => S.edits.managed.push(u), LC_F);
    await page.click(`${sec} [data-me="add"]`);
    await waitText("#pal-list", /No other lighting controller to add/);
    await page.keyboard.press("Escape");
    await page.evaluate((u) => S.edits.managed.splice(S.edits.managed.indexOf(u), 1), LC_F);
    // Ctrl+K picks a member: only the lighting controllers not listed yet.
    await page.keyboard.press("Control+k");
    await page.waitForSelector("#palette:not(.hidden)");
    assert.equal(await page.getAttribute("#pal-input", "placeholder"), "Add a lighting controller…");
    assert.deepEqual(await page.$$eval("#pal-list [role=option] .pt", (n) => n.map((x) => x.textContent)), ["Diele"]);
    await page.keyboard.press("Enter");
    await page.waitForSelector("#palette.hidden", { state: "attached" });
    assert.equal(await page.getAttribute("#pal-input", "placeholder"), "Go to a page or block…");
    // Diele first: the order the app lists them in.
    await page.click(`${sec} [data-me="up"][data-i="1"]`);
    assert.match(await page.textContent(`${sec} .me-list`), /^Diele.*Wohnzimmer/);
    const n0 = edits.length;
    await page.click(`${sec} [data-me="save"]`);
    await until(() => edits.length > n0 && statuses[n0] !== undefined, "the members request");
    assert.equal(statuses[n0], 200);
    assert.deepEqual(edits[n0].ops, [{ op: "members", uuid: CENTRAL, value: [LC_F, LC_W] }]);
    await settled();
    const file = () => readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    assert.match(file(), /^vx_10000050_0000_0500_ffff504f94000001\.members = \(vx_10000041_0000_0410_ffff504f94000001, vx_10000040_0000_0400_ffff504f94000001\)$/m);
    assert.match(file(), /^extern vx_10000050_0000_0500_ffff504f94000001 = CentralLight\(uuid: "10000050-0000-0500-ffff504f94000001"\)$/m);
    await page.waitForFunction(([u, want]) => S.view.objects[u].members.members.map((m) => m.uuid).join() === want, [CENTRAL, `${LC_F},${LC_W}`], { timeout: 10000 });
    await waitText(sec, /edited here[\s\S]*not deployed[\s\S]*Revert/);
    await waitText("#deploy-pend", /members of 1 central block edited here/);
    const changes = await page.evaluate(() => S.view.changes.map((c) => c.text));
    assert.ok(changes.includes("Alle Lichter · member added: Diele"), changes.join(" | "));
    assert.ok(!changes.some((c) => / rec[: ]/.test(c)), changes.join(" | "));
    // Undone and redone through the reply's undo.
    let ops = await undoKey(false, (u) => S.view.objects[u].members.members.length === 1 && !S.edits?.members?.length, CENTRAL);
    assert.deepEqual(ops, [{ op: "members", uuid: CENTRAL, value: null }]);
    assert.doesNotMatch(file(), /members/);
    ops = await undoKey(true, (u) => S.view.objects[u].members.members.length === 2, CENTRAL);
    assert.deepEqual(ops, [{ op: "members", uuid: CENTRAL, value: [LC_F, LC_W] }]);
    // A draft made on a config an undo replaced is dropped, never saved over the newer list.
    await page.waitForSelector(`${sec} [data-me="open"]`);
    await page.click(`${sec} [data-me="open"]`);
    await page.click(`${sec} [data-me="del"][data-i="0"]`);
    ops = await undoKey(false, (u) => S.view.objects[u].members.members.length === 1, CENTRAL);
    assert.deepEqual(ops, [{ op: "members", uuid: CENTRAL, value: null }]);
    await waitText(sec, /The pending config changed while editing: the draft was dropped/);
    assert.equal(await page.$(`${sec} [data-me="save"]`), null);
    // And a save from a draft on an old blob sends nothing.
    const n2 = edits.length;
    await page.evaluate((u) => { meOpen(u); ME.blob = "0000000000000000000000000000000000000000"; meSave(); }, CENTRAL);
    await waitText(sec, /the draft was dropped/);
    assert.equal(edits.length, n2);
    ops = await undoKey(true, (u) => S.view.objects[u].members.members.length === 2, CENTRAL);
    assert.deepEqual(ops, [{ op: "members", uuid: CENTRAL, value: [LC_F, LC_W] }]);
    // Emptied: `()`, and no rec= in the pending config.
    await page.waitForSelector(`${sec} [data-me="open"]`);
    await page.click(`${sec} [data-me="open"]`);
    await page.click(`${sec} [data-me="del"][data-i="1"]`);
    await page.click(`${sec} [data-me="del"][data-i="0"]`);
    await waitText(sec, /No members: saving empties the list/);
    const n1 = edits.length;
    await page.click(`${sec} [data-me="save"]`);
    await until(() => edits.length > n1 && statuses[n1] !== undefined, "the empty members request");
    assert.equal(statuses[n1], 200);
    assert.deepEqual(edits[n1].ops[0].value, []);
    await page.waitForFunction((u) => S.view.objects[u].members.members.length === 0, CENTRAL, { timeout: 10000 });
    await settled();
    assert.match(file(), /^vx_10000050_0000_0500_ffff504f94000001\.members = \(\)$/m);
    // Reverted: vislox's claim goes, the base's list is back, nothing pending.
    await page.waitForSelector(`${sec} [data-me="revert"]`);
    await page.click(`${sec} [data-me="revert"]`);
    await page.waitForFunction((u) => S.view.objects[u].members.members.length === 1 && !S.edits?.members?.length, CENTRAL, { timeout: 15000 });
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    // An alarm's members are shown, never edited.
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, ALARM);
    await waitText(sec, /stay with Loxone Config[\s\S]*1\s*Diele/);
    assert.equal(await page.$(`${sec} [data-me="open"]`), null);
  });
  await step("an NFC Code Touch's access rights are edited as an access set, warned, undone, shared and reverted (#76)", async () => {
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    await page.evaluate(() => editGoNewest());
    await settled();
    const sec = '#props [data-body="Access"]';
    const show = async (u) => {
      await page.evaluate((x) => { gotoPage(S.objPage.get(x), { focus: x }); select(x); }, u);
      await page.waitForFunction((x) => S.sel === x && document.querySelector('#props [data-body="Access"] [data-ac="open"]'), u, { timeout: 15000 });
    };
    await show(TOUCH);
    assert.match(await page.textContent(sec), /Anna \(Always\)[\s\S]*Opens for\s*Anna/);
    // The pickers' lists need the admin token: without it the server refuses them.
    // They come from the config the draft is made on (its blob), never another: a blob the History does not list is
    // refused.
    const url = `http://127.0.0.1:${port}/api/access/choices`;
    const blob = await page.evaluate(() => editRev().blob);
    const ask = (token, base) => fetch(url, { method: "POST", headers: { "Content-Type": "application/json", "X-Vislox": "act", ...(token ? { "X-Vislox-Token": token } : {}) }, body: JSON.stringify({ base }) });
    assert.equal((await ask(null, blob)).status, 401);
    assert.equal((await ask("wrong", blob)).status, 401);
    assert.equal((await ask("secret", "0000000000000000000000000000000000000000")).status, 409);
    const listed = await ask("secret", blob);
    assert.equal(listed.status, 200);
    assert.equal(listed.headers.get("cache-control"), "no-store");
    const listedText = await listed.text();
    assert.match(listedText, /"Anna"/);
    assert.doesNotMatch(listedText, /0A1B2C3D4E5F|00112233/);
    // The draft starts from who holds the right; the pickers list every user, group and access schedule by title.
    await page.click(`${sec} [data-ac="open"]`);
    await page.waitForSelector(`${sec} [data-ac-p="0"]`);
    assert.deepEqual(await page.evaluate(() => AC.rows), [{ kind: "user", uuid: ANNA, schedule: ALWAYS }]);
    assert.deepEqual(await page.$$eval(`${sec} [data-ac-p="0"] option`, (n) => n.map((x) => x.textContent)), ["Anna", "Ben", "Bewohner", "Full access"]);
    assert.deepEqual(await page.$$eval(`${sec} [data-ac-s="0"] option`, (n) => n.map((x) => x.textContent)), ["Always", "Werktags"]);
    // Bewohner instead of Anna, on workdays: Anna loses access, said before anything is sent.
    await page.selectOption(`${sec} [data-ac-p="0"]`, `group|${BEWOHNER}`);
    await page.selectOption(`${sec} [data-ac-s="0"]`, WERKTAGS);
    const n0 = edits.length;
    await page.click(`${sec} [data-ac="save"]`);
    await waitText(sec, /⚠ Anna loses access to Haustür\./);
    assert.equal(await page.textContent(`${sec} [data-ac="save"]`), "Save anyway");
    assert.equal(edits.length, n0);
    await page.click(`${sec} [data-ac="save"]`);
    await until(() => edits.length > n0 && statuses[n0] !== undefined, "the access request");
    assert.equal(statuses[n0], 200);
    assert.deepEqual(edits[n0].ops, [{ op: "access", uuid: TOUCH, value: { grants: [{ kind: "group", uuid: BEWOHNER, schedule: WERKTAGS }] } }]);
    await settled();
    const file = () => readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    assert.match(file(), /^access vx_haustuer_access\n\tgroup "Bewohner": "Werktags"\nend$/m);
    assert.match(file(), /^vx_40000001_0000_0000_ffff504f94000001\.access = vx_haustuer_access$/m);
    await page.waitForFunction((u) => S.view.objects[u].rights.list.map((p) => p.title).join() === "Bewohner", TOUCH, { timeout: 10000 });
    await waitText(sec, /Opens for\s*Ben[\s\S]*edited here[\s\S]*not deployed[\s\S]*Revert/);
    await waitText("#deploy-pend", /access to 1 NFC Code Touch edited here/);
    const changes = await page.evaluate(() => S.view.changes.map((c) => c.text));
    assert.ok(changes.includes("Haustür · access: Anna removed (Always)"), changes.join(" | "));
    assert.ok(changes.includes("Haustür · access: group Bewohner added (Werktags)"), changes.join(" | "));
    // Apply's confirmation says it again, with the claim that does it; nothing is deployed here.
    const confirm = await page.evaluate(async () => rightsApplyText(await pendingHardware(editRev())));
    assert.match(confirm, /⚠ Anna loses access to Haustür \(pages\/vislox\/edits\.loxl:\d+\)/, confirm);
    assert.match(confirm, /Ben gains access to Haustür/, confirm);
    // No credential and no name leaves the review: not in the page title, not in /api/edits.
    assert.doesNotMatch(await page.title(), /Anna|Ben/);
    const ed = await page.evaluate(async () => JSON.stringify(await (await fetch("/api/edits")).json()));
    assert.doesNotMatch(ed, /Anna|Bewohner|Werktags|0A1B2C3D4E5F|00112233/);
    const pageText = await page.textContent("body");
    assert.doesNotMatch(pageText, /0A1B2C3D4E5F|00112233/);
    // Undone and redone through the reply's undo.
    let ops = await undoKey(false, (u) => S.view.objects[u].rights.list.map((p) => p.title).join() === "Anna" && !S.edits?.access?.length, TOUCH);
    assert.deepEqual(ops, [{ op: "access", uuid: TOUCH, value: null }]);
    assert.doesNotMatch(file(), /access/);
    ops = await undoKey(true, (u) => S.view.objects[u].rights.list.map((p) => p.title).join() === "Bewohner", TOUCH);
    assert.deepEqual(ops, [{ op: "access", uuid: TOUCH, value: { set: "vx_haustuer_access", grants: [{ kind: "group", uuid: BEWOHNER, schedule: WERKTAGS }] } }]);
    // Another schedule for Bewohner: Ben keeps the door, but vislox cannot tell the new times cover the old ones, so
    // it is a loss, warned before anything is sent.
    await page.waitForSelector(`${sec} [data-ac="open"]`);
    await page.click(`${sec} [data-ac="open"]`);
    await page.waitForSelector(`${sec} [data-ac-s="0"]`);
    await page.selectOption(`${sec} [data-ac-s="0"]`, ALWAYS);
    const n3 = edits.length;
    await page.click(`${sec} [data-ac="save"]`);
    await waitText(sec, /⚠ Ben may open Haustür only at Always, no longer at Werktags\./);
    assert.equal(await page.textContent(`${sec} [data-ac="save"]`), "Save anyway");
    assert.equal(edits.length, n3);
    // Apply's confirmation warns about such a change on its own (#68): fewer opening times are a loss.
    const narrowed = await page.evaluate((u) => rightsApplyText({ rights: [{ uuid: u, kind: "access", title: "Haustür", lose: [], gain: [], added: [], removed: [], rescheduled: [], narrowed: [{ title: "Ben", uuid: "x", from: ["Werktags"], to: ["Always"] }] }] }), TOUCH);
    assert.match(narrowed, /^⚠ Ben may open Haustür only at Always, no longer at Werktags/, narrowed);
    // A Revert whose deployed rights cannot be read is refused with the reason, never sent as the set's name alone.
    assert.match((await page.evaluate((u) => setRevertValue("access", { deployed: { set: "vx_a", grants: null } }, [], u), TOUCH)).why, /deployed access rights \(vx_a\) cannot be read/);
    // A draft belongs to its Code Touch and its pending config: another selection drops it.
    await show(GARAGE);
    assert.equal(await page.evaluate(() => AC.open), false);
    assert.match(await page.textContent(sec), /No user or group may open it/);
    // Same rights as Haustür: nobody loses access to the garage, so it is claimed at once.
    const n1 = edits.length;
    await page.selectOption(`${sec} [data-ac="same"]`, "vx_haustuer_access");
    await until(() => edits.length > n1 && statuses[n1] !== undefined, "the same-rights request");
    assert.equal(statuses[n1], 200);
    assert.deepEqual(edits[n1].ops, [{ op: "access", uuid: GARAGE, value: { set: "vx_haustuer_access" } }]);
    await settled();
    assert.match(file(), /^vx_40000002_0000_0000_ffff504f94000001\.access = vx_haustuer_access$/m);
    await page.waitForFunction((u) => S.view.objects[u].rights.users.map((p) => p.title).join() === "Ben", GARAGE, { timeout: 10000 });
    // Haustür's editor now changes Garage too, and the warning names both doors.
    await show(TOUCH);
    await waitText(sec, /shared with Garage/);
    await page.click(`${sec} [data-ac="open"]`);
    await page.waitForSelector(`${sec} [data-ac="del"]`);
    await page.click(`${sec} [data-ac="del"][data-i="0"]`);
    await waitText(sec, /No user or group: saving revokes everyone's access/);
    const n2 = edits.length;
    await page.click(`${sec} [data-ac="save"]`);
    await waitText(sec, /⚠ Ben loses access to Haustür; Ben loses access to Garage\./);
    assert.equal(edits.length, n2);
    await page.click(`${sec} [data-ac="cancel"]`);
    // Reverted: Haustür's claim goes (Garage keeps the set), then Garage's; nothing pending.
    await page.waitForSelector(`${sec} [data-ac="revert"]`);
    await page.click(`${sec} [data-ac="revert"]`);
    await page.waitForFunction((u) => S.edits?.access?.length === 1 && S.edits.access[0].uuid === u, GARAGE, { timeout: 15000 });
    await settled();
    await show(GARAGE);
    await page.waitForSelector(`${sec} [data-ac="revert"]`);
    await page.click(`${sec} [data-ac="revert"]`);
    await page.waitForFunction(() => !S.edits?.access?.length, null, { timeout: 15000 });
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
  });
  await step("a push notification's recipients are edited as a set, warned, undone; its message is shown, added and edited (#77)", async () => {
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    await page.evaluate(() => editGoNewest());
    await settled();
    const sec = '#props [data-body="Recipients"]';
    await page.evaluate((u) => select(u), FROST);
    await page.waitForFunction((u) => S.sel === u && document.querySelector('#props [data-body="Recipients"] [data-ac="open"]'), FROST, { timeout: 15000 });
    // Anna gets it directly; Ben as a member of Full access, inherited.
    await waitText(sec, /Anna[\s\S]*Sent to\s*Anna, Ben[\s\S]*Inherited\s*Ben \(Full access\)/);
    // The block that sends it, with its message.
    await waitText('#props [data-body="Message"]', /Sent by\s*Meldungen\s*on: Frost im Garten/);
    // The editor: a user or group per row, no schedule; Full access is said to get it anyway.
    await page.click(`${sec} [data-ac="open"]`);
    await page.waitForSelector(`${sec} [data-ac-p="0"]`);
    assert.deepEqual(await page.evaluate(() => AC.rows.map((r) => [r.kind, r.uuid])), [["user", ANNA]]);
    assert.equal(await page.$(`${sec} [data-ac-s="0"]`), null);
    assert.deepEqual(await page.$$eval(`${sec} [data-ac-p="0"] option`, (n) => n.map((x) => x.textContent)), ["Anna", "Ben", "Bewohner", "Full access (gets every notification anyway)"]);
    // Bewohner instead of Anna: Anna stops getting it, said before anything is sent; Ben keeps it.
    await page.selectOption(`${sec} [data-ac-p="0"]`, `group|${BEWOHNER}`);
    const n0 = edits.length;
    await page.click(`${sec} [data-ac="save"]`);
    await waitText(sec, /⚠ Anna stops getting Frostwarnung\./);
    assert.doesNotMatch(await page.textContent(sec), /Ben stops/);
    assert.equal(edits.length, n0);
    await page.click(`${sec} [data-ac="save"]`);
    await until(() => edits.length > n0 && statuses[n0] !== undefined, "the recipients request");
    assert.equal(statuses[n0], 200);
    assert.deepEqual(edits[n0].ops, [{ op: "recipients", uuid: FROST, value: { recipients: [{ kind: "group", uuid: BEWOHNER }] } }]);
    await settled();
    const file = () => readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    assert.match(file(), /^recipients vx_frostwarnung_recipients\n\tgroup "Bewohner"\nend$/m);
    assert.match(file(), /^vx_40000021_0000_0000_ffff504f94000001\.recipients = vx_frostwarnung_recipients$/m);
    await page.waitForFunction((u) => S.view.objects[u].rights.list.map((p) => p.title).join() === "Bewohner", FROST, { timeout: 10000 });
    await waitText(sec, /edited here[\s\S]*not deployed[\s\S]*Revert/);
    await waitText("#deploy-pend", /recipients of 1 push notification edited here/);
    const changes = await page.evaluate(() => S.view.changes.map((c) => c.text));
    assert.ok(changes.includes("Frostwarnung · recipients: Anna removed"), changes.join(" | "));
    // Apply's confirmation warns with ⚠, naming the claim; nothing is deployed here.
    const confirm = await page.evaluate(async () => rightsApplyText(await pendingHardware(editRev())));
    assert.match(confirm, /⚠ Anna stops getting Frostwarnung \(pages\/vislox\/edits\.loxl:\d+\)/, confirm);
    assert.doesNotMatch(confirm, /Ben stops/, confirm);
    // No name in /api/edits or the page title.
    assert.doesNotMatch(await page.title(), /Anna|Ben/);
    const ed = await page.evaluate(async () => JSON.stringify(await (await fetch("/api/edits")).json()));
    assert.doesNotMatch(ed, /Anna|Bewohner|Frost im Garten/);
    // Undone through the reply's undo: Anna again, no claim.
    const ops = await undoKey(false, (u) => S.view.objects[u].rights.list.map((p) => p.title).join() === "Anna" && !S.edits?.recipients?.length, FROST);
    assert.deepEqual(ops, [{ op: "recipients", uuid: FROST, value: null }]);
    assert.doesNotMatch(file(), /recipients/);
    // Frostwarnung's block on Meldungen: its message, drawn in Loxone Config, so not edited here.
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, WARN);
    const msg = '#props [data-body="Message"]';
    await waitText(msg, /Notification\s*Frostwarnung[\s\S]*On\s*Frost im Garten[\s\S]*Off\s*no message[\s\S]*Minimum time\s*12h[\s\S]*Drawn in Loxone Config/);
    // A push notification added to the page: Regen, with its message; its name is the notification's.
    await page.waitForSelector("#btn-add-block:not([disabled])");
    await page.click("#btn-add-block");
    await page.waitForSelector("#ab-q");
    await page.fill("#ab-q", "push notification");
    await page.press("#ab-q", "Enter");
    await page.waitForSelector('#ab-body [data-ab-field="s:mirrors"]');
    assert.deepEqual(await page.$$eval("#ab-body [data-ab-field]", (n) => n.map((x) => x.dataset.abField)), ["s:mirrors", "s:On", "s:Off", "s:MinimumTime"]);
    assert.deepEqual(await page.$$eval('#ab-body [data-ab-field="s:mirrors"] option', (n) => n.map((x) => x.textContent)), ["Pick a push notification…", "Frostwarnung", "Regen"]);
    // Not sent without a notification, nor with a duration lox would refuse.
    await page.fill('#ab-body [data-ab-field="s:MinimumTime"]', "bald");
    const n1 = edits.length;
    await page.click('#ab-body [data-ab="add"]');
    await waitText("#ab-body", /Pick the push notification it sends[\s\S]*A duration, like 12h/);
    assert.equal(edits.length, n1);
    await page.selectOption('#ab-body [data-ab-field="s:mirrors"]', REGEN);
    await page.dispatchEvent('#ab-body [data-ab-field="s:mirrors"]', "input");
    await page.fill('#ab-body [data-ab-field="s:On"]', "Es regnet");
    await page.fill('#ab-body [data-ab-field="s:MinimumTime"]', "30min");
    await page.click('#ab-body [data-ab="add"]');
    await until(() => edits.length > n1 && statuses[n1] !== undefined, "the add request");
    assert.equal(statuses[n1], 200);
    assert.deepEqual(edits[n1].ops, [{ op: "add_block", page: MELDUNGEN, type: "OutputRefLM", params: {}, settings: { mirrors: REGEN, On: "Es regnet", MinimumTime: "30min" } }]);
    // The new block is selected once the add's reply is handled: that comes after the History shows the compile,
    // so a settled view alone can still be the one before, with the old selection (the CI flake).
    await page.waitForFunction(() => editsBlock(S.sel)?.slug === "vx_outputreflm_1", null, { timeout: 15000 });
    await settled();
    const pf = readFileSync(join(dir, "pages", "vislox", "meldungen.loxl"), "utf8");
    assert.match(pf, /^vx_outputreflm_1 = OutputRefLM\(mirrors: vx_40000022_0000_0000_ffff504f94000001, On: "Es regnet", MinimumTime: 30min\)$/m);
    assert.match(file(), /^extern vx_40000022_0000_0000_ffff504f94000001 = Notification\(uuid: "40000022-0000-0000-ffff504f94000001"\)$/m);
    const added = await page.evaluate(() => S.sel);
    // /api/edits names it as a block of vislox's once it has loaded after the compile.
    await page.waitForFunction((u) => editsBlock(u), added, { timeout: 10000 });
    // A block added here with nothing caps() offers (no `o.edit`) still gets its form, never a TypeError.
    assert.deepEqual(await page.evaluate((u) => editFields({ ...S.view.objects[u], ty: "And", edit: null }).map((f) => f[0]), added), ["title", "s:On", "s:Off", "s:MinimumTime"]);
    await waitText(msg, /Notification\s*Regen[\s\S]*On\s*Es regnet[\s\S]*Minimum time\s*30min/);
    // Its message is edited in its declaration: no name field, the texts and the minimum time.
    await page.click('#props [data-body="Edit"] [data-ed="open"]');
    assert.deepEqual(await page.$$eval("#props [data-ed-field]", (n) => n.map((x) => x.dataset.edField)), ["s:On", "s:Off", "s:MinimumTime"]);
    await page.fill('#props [data-ed-field="s:Off"]', "Trocken");
    await page.fill('#props [data-ed-field="s:MinimumTime"]', "1h");
    const n2 = edits.length;
    await page.click('#props [data-ed="save"]');
    await until(() => edits.length > n2 && statuses[n2] !== undefined, "the message request");
    assert.equal(statuses[n2], 200);
    assert.deepEqual(edits[n2].ops, [{ op: "setting", uuid: added, name: "Off", value: "Trocken" }, { op: "setting", uuid: added, name: "MinimumTime", value: "1h" }]);
    await settled();
    await waitText(msg, /Off\s*Trocken[\s\S]*Minimum time\s*1h/);
    assert.match(readFileSync(join(dir, "pages", "vislox", "meldungen.loxl"), "utf8"), /MinimumTime: 1h, Off: "Trocken"|Off: "Trocken", MinimumTime: 1h|MinimumTime: 1h.*Off: "Trocken"/);
    // Taken out again: nothing pending, the extern of Regen gone with it.
    await page.click('#props [data-ed="remove"]');
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
    await settled();
    assert.doesNotMatch(existsSync(join(dir, "pages", "vislox", "edits.loxl")) ? file() : "", /40000022/);
  });
  await step("a Touch Pure Flex's tiles are shown with lint, edited as a tiles set, reviewed and undone (#78)", async () => {
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
    await page.evaluate(() => editGoNewest());
    await settled();
    const sec = '#props [data-body="Tiles"]';
    await page.evaluate((u) => select(u), FLEX);
    await page.waitForFunction((u) => S.sel === u && document.querySelector('#props [data-body="Tiles"] [data-ti="open"]'), FLEX, { timeout: 15000 });
    // The blocks linked to its API connector: Nord directly, Süd through the OutputRef; Ost is not.
    await waitText(sec, /API-linked\s*Nord[\s\S]*Süd/);
    assert.doesNotMatch(await page.textContent(`${sec} .pr-row:first-child, ${sec} > div:first-child`).catch(() => ""), /Ost/);
    // Tile 3's click toggles Nord, with its text; tile 5's click every linked shade, which lint says on the tile.
    await waitText(sec, /Tile 3[\s\S]*Click\s*SET\(As;Tg;pulse\)\s*·\s*=ECHO\(As\)\s*→\s*Nord/);
    // lox logic lint runs again on the next compile, and then says that tile 5's click names no target.
    writeFileSync(join(dir, "TILELINT"), "");
    // A draft belongs to its device: selecting another block drops it.
    await page.click(`${sec} [data-ti="open"]`);
    await page.waitForSelector(`${sec} [data-ti-api="1|0"]`);
    await page.fill(`${sec} [data-ti-api="1|0"]`, "dropped");
    await page.evaluate((u) => select(u), NORD);
    await page.waitForFunction((u) => S.sel === u, NORD);
    // Nord says which device it is linked to, and which tile addresses it.
    await waitText('#props [data-body="Touch Pure Flex"]', /API-linked to\s*Flex Küche[\s\S]*Addressed by tile 3 click/);
    await page.evaluate((u) => select(u), FLEX);
    await page.waitForFunction((u) => S.sel === u && !TI.open && document.querySelector('#props [data-body="Tiles"] [data-ti="open"]'), FLEX);
    // Tile 3's click closes Süd instead of Nord, and its arrow up opens Süd. The targets offered are the linked blocks.
    await page.click(`${sec} [data-ti="open"]`);
    await page.waitForSelector(`${sec} [data-ti-api="1|0"]`);
    assert.equal(await page.inputValue(`${sec} [data-ti-api="1|0"]`), "SET(As;Tg;pulse)");
    assert.deepEqual(await page.$$eval(`${sec} [data-ti-to="1|0"]`, (n) => n.map((x) => x.parentElement.textContent.trim())), ["Nord", "Süd"]);
    await page.fill(`${sec} [data-ti-api="1|0"]`, "SET(As;Cc;pulse)");
    await page.uncheck(`${sec} [data-ti-to="1|0"][value="${NORD}"]`);
    await page.check(`${sec} [data-ti-to="1|0"][value="${SUED}"]`);
    await page.fill(`${sec} [data-ti-api="1|1"]`, "SET(As;Co;pulse)");
    await page.check(`${sec} [data-ti-to="1|1"][value="${SUED}"]`);
    const n0 = edits.length;
    await page.click(`${sec} [data-ti="save"]`);
    await until(() => edits.length > n0 && statuses[n0] !== undefined, "the tiles request");
    assert.equal(statuses[n0], 200);
    // Only the tile the draft changed: the others keep their programs.
    assert.deepEqual(edits[n0].ops, [{ op: "tiles", uuid: FLEX, value: { tiles: [{ index: 3, click: { api: "SET(As;Cc;pulse)", text: "=ECHO(As)", to: [SUED] }, up: { api: "SET(As;Co;pulse)", to: [SUED] } }] } }]);
    assert.equal(edits[n0].base, await page.evaluate(() => TI.blob ?? editRev()?.blob) ?? edits[n0].base);
    await settled();
    const file = () => readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    const sued = "vx_70000012_0000_0000_ffff504f94000001";
    assert.match(file(), /^extern vx_70000001_0000_0000_ffff504f94000001 = TreeDevice\(uuid: "70000001-0000-0000-ffff504f94000001"\)$/m);
    assert.match(file(), new RegExp(`^extern ${sued} = AutoJalousie\\(uuid: "${SUED}"\\)$`, "m"));
    assert.match(file(), /^tiles vx_flex_kueche_tiles\n\ttile 3\($/m);
    for (const l of ['\t\tclick: "SET(As;Cc;pulse)",', '\t\tup: "SET(As;Co;pulse)",', `\t\tto: ${sued},`, '\t\tclick_text: "=ECHO(As)",']) assert.ok(file().includes(`${l}\n`), `${l} in\n${file()}`);
    assert.match(file(), /^vx_70000001_0000_0000_ffff504f94000001\.tiles = vx_flex_kueche_tiles$/m);
    assert.doesNotMatch(file(), /70000011/);
    // Compiled: the pending config has the new tile, reviewed per function.
    await page.waitForFunction((u) => S.view.objects[u].tiles.tiles[1].functions[0].api === "SET(As;Cc;pulse)", FLEX, { timeout: 10000 });
    await waitText(sec, /edited here[\s\S]*not deployed[\s\S]*Revert/);
    await waitText(sec, /Written by vislox as vx_flex_kueche_tiles/);
    await page.waitForFunction(() => S.check?.lint?.findings?.some((f) => f.kind === "tile-broadcast"), null, { timeout: 15000 });
    await waitText(sec, /Tile 5[\s\S]*⚠ Lint \(tile-broadcast\): tile 5 click: `SET\(As;Tg;pulse\)` names no target/);
    assert.doesNotMatch(await page.textContent(sec), /Tile 3[^⚠]*⚠ Lint[\s\S]*Tile 5/);
    rmSync(join(dir, "TILELINT"));
    await waitText("#deploy-pend", /tiles of 1 Touch Pure Flex edited here/);
    const changes = await page.evaluate(() => S.view.changes.map((c) => c.text));
    for (const c of ['Flex Küche · tile 3 click: "SET(As;Tg;pulse)" → "SET(As;Cc;pulse)"', "Flex Küche · tile 3 click targets: Nord → Süd", 'Flex Küche · tile 3 up: — → "SET(As;Co;pulse)"', "Flex Küche · tile 3 up targets: — → Süd"])
      assert.ok(changes.includes(c), changes.join(" | "));
    // Undone through the reply's undo: the claim goes, and tile 3 is as Loxone Config has it.
    const ops = await undoKey(false, (u) => S.view.objects[u].tiles.tiles[1].functions[0].api === "SET(As;Tg;pulse)" && !S.edits?.tiles?.length, FLEX);
    assert.deepEqual(ops, [{ op: "tiles", uuid: FLEX, value: null }]);
    assert.doesNotMatch(existsSync(join(dir, "pages", "vislox", "edits.loxl")) ? file() : "", /tiles|70000012/);
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
  });
  await step("block settings read as Lox Logic writes them; a constant takes its unit (#79)", async () => {
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
    await page.evaluate(() => editGoNewest());
    await settled();
    const sets = '#props [data-body="Settings"]';
    // Drawn in Loxone Config: shown by their Lox Logic names, read-only until the module manages the block.
    await pick(PULSE);
    await waitText(sets, /At\s*18:58[\s\S]*Pulse\s*time[\s\S]*Read-only[\s\S]*Adopt into vislox/);
    assert.doesNotMatch(await page.textContent('#props [data-body="Advanced"]').catch(() => ""), /\bSec\b|\bTyp\b/);
    await pick(HUE);
    await waitText(sets, /Program\s*1 line/);
    // A constant as Lox Logic writes it: 1800 s is 30min, and the form starts from that.
    await pick(NACH);
    await waitText('#props [data-body="Parameters"]', /Time\s*30min/);
    await page.click('#props [data-ed="open"]');
    assert.equal(await page.inputValue(field("p:Time")), "30min");
    // A unit the port is not in: the server says so, and nothing changes.
    await page.fill(field("p:Time"), "70%");
    await page.press(field("p:Time"), "Enter");
    await waitText("#props", /Time: `70%`: the port is in seconds/);
    await page.fill(field("p:Time"), "1h");
    const n = edits.length;
    await page.press(field("p:Time"), "Enter");
    await until(() => edits.length > n && statuses[n] !== undefined, "the param request");
    assert.equal(statuses[n], 200);
    assert.deepEqual(edits[n].ops, [{ op: "param", uuid: NACH, port: "Time", value: "1h" }]);
    await settled();
    const claims = () => readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    assert.match(claims(), /^vx_50000003_0000_0000_ffff504f94000001\.Time = 1h$/m);
    await page.waitForFunction((u) => S.view.objects[u].ports.find((p) => p.key === "Time").def === "3600", NACH, { timeout: 10000 });
    await waitText('#props [data-body="Parameters"]', /Time\s*30min\s*1h/);
    await waitText('#props [data-body="Edit"]', /Time\s*1h\s*not deployed/);
    assert.ok((await page.evaluate(() => S.view.changes.map((c) => c.text))).includes("Nachlauf Zeit · Time: 30min → 1h"));
    const ops = await undoKey(false, (u) => S.view.objects[u].ports.find((p) => p.key === "Time").def === "1800", NACH);
    assert.deepEqual(ops, [{ op: "param", uuid: NACH, port: "Time", value: null }]);
  });
  await step("a managed block's settings and program are edited, reviewed as Lox Logic writes them, and undone (#79)", async () => {
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
    const sets = '#props [data-body="Settings"]';
    const zeiten = () => readFileSync(join(dir, "pages", "vislox", "zeiten.loxl"), "utf8");
    const adopt = async (u, slug) => {
      await pick(u);
      await page.waitForSelector('#props [data-ed="adopt-open"]:not([disabled])');
      await page.click('#props [data-ed="adopt-open"]');
      await page.fill("#ad-slug", slug);
      await page.click('#props [data-ed="adopt"]');
      await page.waitForFunction((x) => S.edits?.blocks?.some((b) => b.uuid === x && b.adopted), u, { timeout: 20000 });
      await settled();
    };
    // Adopted, the PulseAt's time is edited in its declaration.
    await adopt(PULSE, "vx_abends");
    await page.waitForFunction(() => !/Read-only/.test(document.querySelector('#props [data-body="Settings"]')?.textContent || ""), null, { timeout: 10000 });
    await page.click('#props [data-ed="open"]');
    const fields = await page.$$eval("#props [data-ed-field]", (x) => x.map((f) => f.dataset.edField));
    assert.ok(fields.includes("s:At") && fields.includes("s:Pulse") && fields.includes("s:Desc"), fields.join(" "));
    await page.fill(field("s:At"), "06:58");
    let n = edits.length;
    await page.press(field("s:At"), "Enter");
    await until(() => edits.length > n && statuses[n] !== undefined, "the At request");
    assert.equal(statuses[n], 200);
    assert.deepEqual(edits[n].ops, [{ op: "setting", uuid: PULSE, name: "At", value: "06:58" }]);
    await settled();
    assert.match(zeiten(), /^vx_abends = PulseAt\("Abends", At: 06:58\)$/m);
    await waitText(sets, /At\s*18:58\s*06:58/);
    assert.ok((await page.evaluate(() => S.view.changes.map((c) => c.text))).includes("Abends · At: 18:58 → 06:58"));
    await undoKey(false, (u) => S.view.objects[u].settings.find((x) => x.name === "At").value === "18:58", PULSE);
    await undoKey(false, (u) => !S.edits?.blocks?.some((b) => b.uuid === u), PULSE);
    // Adopted, the program block's program is edited in a file of its own.
    await adopt(HUE, "vx_hue");
    await page.click('#props [data-ed="open"]');
    await page.waitForSelector('#props textarea[data-ed-field="s:Code"]');
    // It starts from the program the block runs.
    assert.equal(await page.inputValue(field("s:Code")), "setoutput(0, 1);");
    // A program Loxone Config wrote with CRLF, saved unchanged (the text area shows LF): nothing is sent, and the
    // program stays Loxone Config's.
    const code = await page.evaluate((u) => S.view.objects[u].settings.find((x) => x.name === "Code").value, HUE);
    await page.evaluate((u) => { S.view.objects[u].settings.find((x) => x.name === "Code").value = "setoutput(0, 1);\r\nsetoutput(2, 0);"; }, HUE);
    await page.click('#props [data-ed="cancel"]');
    await page.click('#props [data-ed="open"]');
    await page.fill(field("s:Code"), "setoutput(0, 1);\nsetoutput(2, 0);");
    n = edits.length;
    await page.click('#props [data-ed="save"]');
    await waitText("#props", /Nothing changed\./);
    assert.equal(edits.length, n);
    await page.click('#props [data-ed="cancel"]');
    await page.evaluate(([u, c]) => { S.view.objects[u].settings.find((x) => x.name === "Code").value = c; }, [HUE, code]);
    await page.click('#props [data-ed="open"]');
    assert.equal(await page.inputValue(field("s:Code")), "setoutput(0, 1);");
    // Enter is a line break in a program; Ctrl+Enter saves.
    await page.click(field("s:Code"));
    await page.keyboard.press("End");
    await page.keyboard.press("Enter");
    await page.keyboard.type("setoutput(1, 0);");
    n = edits.length;
    await page.keyboard.press("Control+Enter");
    await until(() => edits.length > n && statuses[n] !== undefined, "the program request");
    assert.equal(statuses[n], 200);
    assert.deepEqual(edits[n].ops, [{ op: "setting", uuid: HUE, name: "Code", value: "setoutput(0, 1);\nsetoutput(1, 0);" }]);
    await settled();
    assert.match(zeiten(), /^vx_hue = Code16\("Hue", Code: file\("vx_hue\.c"\)\)$/m);
    assert.equal(readFileSync(join(dir, "pages", "vislox", "vx_hue.c"), "utf8"), "setoutput(0, 1);\nsetoutput(1, 0);");
    // Reviewed as a line diff, in Properties and the change list; the Edit section names it by its size.
    await page.waitForSelector(`${sets} .prog-diff .pd.new`);
    assert.match(await page.textContent(`${sets} .prog-diff`), /\+ setoutput\(1, 0\);/);
    await waitText('#props [data-body="Edit"]', /Program\s*edited here, 2 lines\s*not deployed/);
    const changes = await page.evaluate(() => S.view.changes.map((c) => c.text));
    assert.ok(changes.some((c) => /^Hue · program: .*\+ setoutput\(1, 0\);/.test(c)), changes.join(" | "));
    // A program that starts with a blank line keeps it, saved and shown again in the form.
    await page.click('#props [data-ed="open"]');
    assert.equal(await page.inputValue(field("s:Code")), "setoutput(0, 1);\nsetoutput(1, 0);");
    await page.fill(field("s:Code"), "\nsetoutput(0, 1);\nsetoutput(1, 0);");
    n = edits.length;
    await page.click('#props [data-ed="save"]');
    await until(() => edits.length > n && statuses[n] !== undefined, "the blank-line request");
    assert.equal(statuses[n], 200);
    assert.equal(edits[n].ops[0].value, "\nsetoutput(0, 1);\nsetoutput(1, 0);");
    await settled();
    assert.equal(readFileSync(join(dir, "pages", "vislox", "vx_hue.c"), "utf8"), "\nsetoutput(0, 1);\nsetoutput(1, 0);");
    await page.waitForFunction((u) => S.edits?.blocks?.find((b) => b.uuid === u)?.settings?.find((x) => x.name === "Code")?.value?.startsWith("\n"), HUE, { timeout: 10000 });
    await page.click('#props [data-ed="open"]');
    assert.equal(await page.inputValue(field("s:Code")), "\nsetoutput(0, 1);\nsetoutput(1, 0);");
    await page.click('#props [data-ed="cancel"]');
    await undoKey(false, (u) => S.view.objects[u].settings.find((x) => x.name === "Code").value === "setoutput(0, 1);\nsetoutput(1, 0);", HUE);
    // Undone: the binding and its file go, and the block runs Loxone Config's program again.
    const ops = await undoKey(false, (u) => S.view.objects[u].settings.find((x) => x.name === "Code").value === "setoutput(0, 1);", HUE);
    assert.deepEqual(ops, [{ op: "setting", uuid: HUE, name: "Code", value: null }]);
    assert.equal(existsSync(join(dir, "pages", "vislox", "vx_hue.c")), false);
    await undoKey(false, (u) => !S.edits?.blocks?.some((b) => b.uuid === u), HUE);
  });
  await step("a managed block is renamed keeping its UUIDs, deleted, and brought back with them (#80)", async () => {
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
    const zeiten = () => readFileSync(join(dir, "pages", "vislox", "zeiten.loxl"), "utf8");
    const lockNow = () => JSON.parse(readFileSync(join(dir, "logic.lock.json"), "utf8"));
    const outWas = readFileSync(join(dir, "out.Loxone"), "utf8");
    await pick(NACH);
    await page.waitForSelector('#props [data-ed="adopt-open"]:not([disabled])');
    await page.click('#props [data-ed="adopt-open"]');
    await page.fill("#ad-slug", "vx_nach");
    await page.click('#props [data-ed="adopt"]');
    await page.waitForFunction((x) => S.edits?.blocks?.some((b) => b.uuid === x && b.adopted), NACH, { timeout: 20000 });
    await settled();
    await waitText('#props [data-body="Edit"]', /Slug\s*vx_nach\s*Rename\s*Delete/);
    // Renamed: a slug that is not vislox's is refused in the browser, nothing sent.
    await page.click('#props [data-ed="rename-open"]');
    assert.equal(await page.inputValue("#rn-slug"), "vx_nach");
    let n = edits.length;
    await page.fill("#rn-slug", "Nachlauf");
    await page.press("#rn-slug", "Enter");
    await waitText("#props", /A slug is vx_ and then lowercase letters/);
    assert.equal(edits.length, n);
    // One the server refuses says why, and the form stays open.
    await page.fill("#rn-slug", "vx_abc__d");
    await page.press("#rn-slug", "Enter");
    await until(() => statuses[n] !== undefined, "the refused rename");
    assert.equal(statuses[n], 422);
    await waitText("#props", /lox's for the blocks an expression makes/);
    n = edits.length;
    await page.fill("#rn-slug", "vx_nachlauf");
    await page.press("#rn-slug", "Enter");
    await page.waitForFunction((x) => S.edits?.blocks?.some((b) => b.uuid === x && b.slug === "vx_nachlauf"), NACH, { timeout: 20000 });
    await settled();
    assert.deepEqual(edits[n].ops, [{ op: "rename", uuid: NACH, slug: "vx_nachlauf" }]);
    await waitText("#props", /Renamed to vx_nachlauf: lox keeps its UUIDs, and the config does not change/);
    assert.match(zeiten(), /^vx_nachlauf = Monoflop\("Nachlauf Zeit"\)$/m);
    assert.match(zeiten(), /^moved vx_nach -> vx_nachlauf$/m);
    assert.equal(lockNow().objects.vx_nachlauf.uuid, NACH);
    assert.equal(lockNow().objects.vx_nach, undefined);
    // The config is what it was: nothing pending.
    assert.equal(readFileSync(join(dir, "out.Loxone"), "utf8"), outWas);
    await waitText('[data-body="Lox Logic"]', /vx_nachlauf/);
    // The line is done once compiled: the Deploy pane offers to drop it, and the drop is an edit of its own.
    await waitText("#deploy-lines", /1 line of vislox's files lox needs no more\s*[\s\S]*moved vx_nach -> vx_nachlauf/);
    n = edits.length;
    await page.click("#btn-tidy");
    await page.waitForFunction(() => !S.edits?.stale_lines?.length, null, { timeout: 20000 });
    await settled();
    assert.deepEqual(edits[n].ops, [{ op: "tidy" }]);
    assert.doesNotMatch(zeiten(), /moved/);
    await page.waitForSelector("#deploy-lines", { state: "hidden" });
    // Undone (the tidy first, which changes nothing then), the slug is back, and lox is told again.
    await undoKey(false, () => true);
    let ops = await undoKey(false, (x) => S.edits?.blocks?.some((b) => b.uuid === x && b.slug === "vx_nach"), NACH);
    assert.deepEqual(ops, [{ op: "rename", uuid: NACH, slug: "vx_nach" }]);
    assert.match(zeiten(), /^moved vx_nachlauf -> vx_nach$/m);
    assert.equal(lockNow().objects.vx_nach.uuid, NACH);

    // Deleted: lox takes it out of the config with a tombstone, so the review shows it removed.
    await pick(NACH);
    await page.waitForSelector('#props [data-ed="delete"]:not([disabled])');
    n = edits.length;
    await page.click('#props [data-ed="delete"]');
    await page.waitForFunction((u) => S.rev?.startsWith("pending:") && S.viewRev === S.rev && S.view?.objects[u]?.status === "removed", NACH, { timeout: 20000 });
    await settled();
    assert.deepEqual(edits[n].ops, [{ op: "remove_block", uuid: NACH }]);
    assert.match(zeiten(), /^removed vx_nach$/m);
    assert.doesNotMatch(zeiten(), /= Monoflop/);
    assert.ok(!readFileSync(join(dir, "out.Loxone"), "utf8").includes(NACH));
    assert.equal(lockNow().removed[NACH].slug, "vx_nach");
    // Undo brings it back with its UUIDs: the lock entry, not a new block.
    ops = await undoKey(false, (u) => !!S.edits?.blocks?.some((b) => b.uuid === u), NACH);
    assert.equal(ops[0].op, "restore_block");
    assert.equal(ops[0].uuid, NACH);
    assert.match(zeiten(), /^vx_nach = Monoflop\("Nachlauf Zeit"\)$/m);
    assert.doesNotMatch(zeiten(), /removed/);
    assert.equal(lockNow().objects.vx_nach.uuid, NACH);
    assert.equal(lockNow().removed?.[NACH], undefined);
    assert.equal(readFileSync(join(dir, "out.Loxone"), "utf8"), outWas);
    // Redo deletes it again; undo once more, then hand it back.
    ops = await undoKey(true, (u) => !S.edits?.blocks?.some((b) => b.uuid === u), NACH);
    assert.deepEqual(ops, [{ op: "remove_block", uuid: NACH }]);
    await undoKey(false, (u) => !!S.edits?.blocks?.some((b) => b.uuid === u), NACH);
    ops = await undoKey(false, (u) => !S.edits?.blocks?.some((b) => b.uuid === u), NACH);
    assert.deepEqual(ops, [{ op: "unadopt", uuids: [NACH] }]);
    assert.equal(readFileSync(join(dir, "out.Loxone"), "utf8"), outWas);
    assert.equal(lockNow().objects.vx_nach, undefined);
  });
  await step("let constants are shown, made, bound and changed in every port at once, and undone (#81)", async () => {
    await settled();
    const params = '#props [data-body="Parameters"]', edit = '#props [data-body="Edit"]';
    const claims = () => readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    const time = (u) => `S.view.objects["${u}"].ports.find((p) => p.key === "Time").def`;
    const send = async (act, what) => {
      const n = edits.length;
      await act();
      await until(() => edits.length > n && statuses[n] !== undefined, what);
      assert.equal(statuses[n], 200, what);
      await settled();
      return edits[n].ops;
    };
    // The module's own constant: its name beside the value. The constant is changed on its line in the module's
    // file and nothing else (its line named as the server listed it); only this block is off, the module's file
    // binding the port (the server says why).
    const konstPath = join(dir, "pages", "konst.loxl"), konst = readFileSync(konstPath, "utf8");
    await pick(K1);
    await page.waitForFunction((u) => S.edits?.constants?.uses?.[u]?.Time, K1);
    await waitText(params, /Time\s*30min \(nachlauf\)/);
    assert.match(await page.getAttribute(`${params} .const-of`, "title"), /nachlauf = 30min \(pages\/konst\.loxl:1\)/);
    await page.click('#props [data-ed="open"]');
    assert.ok(!(await page.isDisabled(field("p:Time"))));
    await waitText(edit, /From nachlauf \(pages\/konst\.loxl:1\), 1 use\. vislox rewrites its value on pages\/konst\.loxl:1 and nothing else/);
    await waitText(edit, /the module's own file binds it \(pages\/konst\.loxl:3\)/);
    assert.ok(await page.isDisabled('#props [data-ed-scope="p:Time"][value="block"]'));
    assert.ok(await page.isChecked('#props [data-ed-scope="p:Time"][value="constant"]'));
    await page.fill(field("p:Time"), "45min");
    let ops = await send(() => page.click('#props [data-ed="save"]'), "change the module's constant");
    assert.deepEqual(ops, [{ op: "let", name: "nachlauf", value: "45min", source: "let nachlauf = 30min" }]);
    assert.equal(readFileSync(konstPath, "utf8"), konst.replace("let nachlauf = 30min", "let nachlauf = 45min"));
    await page.waitForFunction(`${time(K1)} === "2700"`, null, { timeout: 10000 });
    // Not deployed: under Parameters with Revert, in the Deploy pane and in Apply's review.
    await waitText(params, /Constant nachlauf\s*45min\s*not deployed\s*Revert/);
    await waitText("#deploy-pend", /the constant nachlauf \(1 use\)/);
    assert.match(await page.evaluate(() => constApplyText()), /let nachlauf = 45min \(was 30min\), pages\/konst\.loxl:1: 1 use/);
    // Undone: the module's file has its exact old bytes again.
    ops = await undoKey(false, `${time(K1)} === "1800"`);
    assert.deepEqual(ops, [{ op: "let", name: "nachlauf", value: "30min", source: "let nachlauf = 45min" }]);
    assert.equal(readFileSync(konstPath, "utf8"), konst);
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
    // Make a constant of K2's time: `let vx_time = 30min`, and K2's Time bound to it, in one request.
    await pick(K2);
    await page.click('#props [data-ed="open"]');
    await page.click('#props [data-ed="mk-open"][data-field="p:Time"]');
    assert.equal(await page.inputValue("#ed-mk"), "vx_time");
    ops = await send(() => page.press("#ed-mk", "Enter"), "make a constant");
    assert.deepEqual(ops, [{ op: "let", name: "vx_time", value: "30min" }, { op: "param", uuid: K2, port: "Time", value: "vx_time" }]);
    assert.match(claims(), /^let vx_time = 30min$/m);
    assert.match(claims(), new RegExp(`^vx_${K2.replace(/-/g, "_")}\\.Time = vx_time$`, "m"));
    await page.waitForFunction((u) => S.edits?.constants?.uses?.[u]?.Time?.name === "vx_time", K2, { timeout: 10000 });
    await waitText(params, /Time\s*30min \(vx_time\)/);
    // K3 takes it by its name.
    await pick(K3);
    await page.click('#props [data-ed="open"]');
    await page.fill(field("p:Time"), "vx_time");
    ops = await send(() => page.press(field("p:Time"), "Enter"), "bind K3");
    assert.deepEqual(ops, [{ op: "param", uuid: K3, port: "Time", value: "vx_time" }]);
    await page.waitForFunction(`${time(K3)} === "1800"`, null, { timeout: 10000 });
    // Applied (as Apply leaves it: the base is the compile, committed), so the constant has a deployed value.
    writeFileSync(join(dir, "base.Loxone"), readFileSync(join(dir, "out.Loxone")));
    git("add", "-A");
    git("commit", "-q", "-m", "Applied the constant");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || "") &&
      (S.edits?.claims || []).some((c) => c.op === "let" && c.deployed === "30min" && !c.pending), null, { timeout: 20000 });
    await settled();
    // Changed on K2, it asks: the constant (2 uses) or only this block. The constant changes both ports.
    await pick(K2);
    await page.waitForFunction(() => S.edits?.constants?.lets?.find((l) => l.name === "vx_time")?.uses === 2, null, { timeout: 10000 });
    await page.click('#props [data-ed="open"]');
    await waitText(edit, /Change the constant \(2 uses\)[\s\S]*Only this block/);
    await page.fill(field("p:Time"), "45min");
    ops = await send(() => page.click('#props [data-ed="save"]'), "change the constant");
    assert.deepEqual(ops, [{ op: "let", name: "vx_time", value: "45min" }]);
    await page.waitForFunction(`${time(K2)} === "2700" && ${time(K3)} === "2700"`, null, { timeout: 10000 });
    // The Changes list groups the two under the constant, next to each other.
    await waitText("#changes", /let vx_time = 45min · 2 changes/);
    const list = await page.$$eval("#changes > *", (els) => els.map((e) => [e.className, e.textContent]));
    const at = list.findIndex(([c]) => c === "chg-grp");
    assert.match(list[at][1], /pages\/vislox\/edits\.loxl:\d+/);
    assert.deepEqual(list.slice(at + 1, at + 3).map(([c, t]) => [c.includes("in-grp"), /Keller|Garten/.test(t)]), [[true, true], [true, true]]);
    // The constant is a claim not deployed: Revert puts back its deployed value, in both ports.
    await waitText(edit, /Constant vx_time\s*45min\s*not deployed\s*Revert/);
    ops = await send(() => page.click(`#props [data-ed="revert"][data-key="let||vx_time"]`), "revert the constant");
    assert.deepEqual(ops, [{ op: "let", name: "vx_time", value: "30min" }]);
    await page.waitForFunction(`${time(K2)} === "1800" && ${time(K3)} === "1800"`, null, { timeout: 10000 });
    assert.match(claims(), /^let vx_time = 30min$/m);
    // Only this block: K3's own value; the constant stays.
    await pick(K3);
    await page.click('#props [data-ed="open"]');
    await page.check('#props [data-ed-scope="p:Time"][value="block"]');
    await page.fill(field("p:Time"), "20min");
    ops = await send(() => page.click('#props [data-ed="save"]'), "only this block");
    assert.deepEqual(ops, [{ op: "param", uuid: K3, port: "Time", value: "20min" }]);
    await page.waitForFunction(`${time(K3)} === "1200" && ${time(K2)} === "1800"`, null, { timeout: 10000 });
    // Undone step by step, back to what is deployed.
    ops = await undoKey(false, `${time(K3)} === "1800"`);
    assert.deepEqual(ops, [{ op: "param", uuid: K3, port: "Time", value: "vx_time" }]);
    ops = await undoKey(false, `${time(K2)} === "2700" && ${time(K3)} === "2700"`);
    assert.deepEqual(ops, [{ op: "let", name: "vx_time", value: "45min" }]);
    ops = await undoKey(false, `${time(K2)} === "1800" && ${time(K3)} === "1800"`);
    assert.deepEqual(ops, [{ op: "let", name: "vx_time", value: "30min" }]);
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
    assert.match(claims(), /^let vx_time = 30min$/m);
    // A default: Default… on Kurz's Time (a block the module manages) lists the blocks it changes (Lang sets its
    // own), then makes it in vislox's file, with exactly those blocks as the consent.
    await pick(K4);
    await page.click(`${params} [data-dflt="open"][data-key="Time"]`);
    await waitText(params, /a new statement in pages\/vislox\/edits\.loxl\. It changes every Monoflop that does not set Time itself, the module's own blocks among them: 1 block: Kurz\./);
    await page.fill("#dflt-v", "5min");
    // The blocks change while the panel is open (a Monoflop added to the module's file): the save is refused, and the
    // panel asks the server again and shows the new list for consent; so again once the file is put back.
    const zeiten = join(dir, "pages", "zeiten.loxl"), zeitenWas = readFileSync(zeiten, "utf8");
    const refused = async (re) => {
      const n = edits.length;
      await page.click(`${params} [data-dflt="save"]`);
      await until(() => edits.length > n && statuses[n] !== undefined, "a refused default");
      assert.equal(statuses[n], 422);
      await waitText(params, re);
    };
    writeFileSync(zeiten, zeitenWas + 'mittel = Monoflop("Mittel")\n');
    await refused(/not the ones listed before: now 2 blocks: Kurz, mittel\. Save again to confirm these/);
    writeFileSync(zeiten, zeitenWas);
    await refused(/not the ones listed before: now 1 block: Kurz\. Save again to confirm these/);
    ops = await send(() => page.click(`${params} [data-dflt="save"]`), "make a default");
    assert.deepEqual(ops, [{ op: "defaults", type: "Monoflop", port: "Time", value: "5min", blocks: ["kurz"] }]);
    assert.match(claims(), /^defaults Monoflop\(Time: 5min\)$/m);
    await page.waitForFunction(`${time(K4)} === "300" && ${time(K5)} === "3600"`, null, { timeout: 10000 });
    await waitText(params, /Time\s*1min\s*5min \(defaults Monoflop\)/);
    await waitText(params, /Default Monoflop Time\s*5min\s*not deployed/);
    await waitText("#deploy-pend", /the default Monoflop Time, which changes 1 block \(Kurz\)/);
    assert.match(await page.evaluate(() => constApplyText()), /defaults Monoflop\(Time: 5min\) \(was none\), pages\/vislox\/edits\.loxl:\d+: changes 1 block: Kurz/);
    // Revert, named by kind, type and port (not by its place in the list): the default is taken out; undone, it is back.
    ops = await send(() => page.click(`${params} [data-dflt="revert"][data-key="defaults|Monoflop|Time"]`), "revert the default");
    assert.deepEqual(ops, [{ op: "defaults", type: "Monoflop", port: "Time", value: null, blocks: ["kurz"] }]);
    await page.waitForFunction(`${time(K4)} === "60"`, null, { timeout: 10000 });
    assert.doesNotMatch(claims(), /defaults/);
    ops = await undoKey(false, `${time(K4)} === "300"`);
    assert.deepEqual(ops, [{ op: "defaults", type: "Monoflop", port: "Time", value: "5min", blocks: ["kurz"] }]);
    // Changed again: vislox's own statement, so no line is named.
    await page.click(`${params} [data-dflt="open"][data-key="Time"]`);
    await waitText(params, /vislox's statement in pages\/vislox\/edits\.loxl:\d+/);
    assert.equal(await page.inputValue("#dflt-v"), "5min");
    await page.fill("#dflt-v", "10min");
    ops = await send(() => page.press("#dflt-v", "Enter"), "change the default");
    assert.deepEqual(ops, [{ op: "defaults", type: "Monoflop", port: "Time", value: "10min", blocks: ["kurz"] }]);
    await page.waitForFunction(`${time(K4)} === "600"`, null, { timeout: 10000 });
    // Undone twice: the statement is gone, and Kurz has Loxone Config's value again.
    ops = await undoKey(false, `${time(K4)} === "300"`);
    assert.deepEqual(ops, [{ op: "defaults", type: "Monoflop", port: "Time", value: "5min", blocks: ["kurz"] }]);
    ops = await undoKey(false, `${time(K4)} === "60"`);
    assert.deepEqual(ops, [{ op: "defaults", type: "Monoflop", port: "Time", value: null, blocks: ["kurz"] }]);
    assert.doesNotMatch(claims(), /defaults/);
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
  });
  await step("the defaults line and an open palette name blocks from the view that is shown (#126)", async () => {
    await settled();
    const params = '#props [data-body="Parameters"]';
    const claims = () => readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    const zeiten = join(dir, "pages", "zeiten.loxl"), zeitenWas = readFileSync(zeiten, "utf8");
    const n = edits.length;
    await pick(K4);
    await page.click(`${params} [data-dflt="open"][data-key="Time"]`);
    await page.fill("#dflt-v", "5min");
    await page.click(`${params} [data-dflt="save"]`);
    await until(() => edits.length > n && statuses[n] !== undefined, "a default");
    assert.equal(statuses[n], 200);
    await settled();
    await waitText("#deploy-pend", /the default Monoflop Time, which changes 1 block \(Kurz\)/);
    // The module renames the block and is compiled: until the view of that compile is in, the line says the
    // slug (the view shown is the one before); then the new name, never the old one in between.
    writeFileSync(zeiten, zeitenWas.replace('Monoflop("Kurz")', 'Monoflop("Kurz neu")'));
    await waitText("#deploy-check", /Pending is from before the latest edit to pages\/zeiten\.loxl/);
    const route = await viewRoute("hold");
    try {
      await page.click("#btn-compile");
      await page.waitForFunction(() => S.rev.startsWith("pending:") && S.viewRev !== S.rev, null, { timeout: 10000 });
      await waitText("#deploy-pend", /the default Monoflop Time, which changes 1 block \(kurz\)/);
      // A palette opened meanwhile lists the names of the view shown; it is refreshed when the new view is in.
      await page.evaluate(() => openPalette("kurz"));
      await waitText("#pal-list", /Kurz/);
      assert.doesNotMatch(await page.textContent("#pal-list"), /Kurz neu/);
    } finally { await route.stop(); }
    await waitText("#deploy-pend", /the default Monoflop Time, which changes 1 block \(Kurz neu\)/);
    await waitText("#pal-list", /Kurz neu/);
    await page.evaluate(() => closePalette());
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
    assert.doesNotMatch(claims(), /defaults/);
    writeFileSync(zeiten, zeitenWas);
    await settled();
  });
  await step("expressions are drawn as one block, shown with their source, edited, named and undone (#82)", async () => {
    await settled();
    const ex = '#props [data-body="Expression"]';
    const claims = () => readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    const send = async (act, what) => {
      const n = edits.length;
      await act();
      await until(() => edits.length > n && statuses[n] !== undefined, what);
      assert.equal(statuses[n], 200, what);
      await settled();
      return edits[n].ops;
    };
    const go = async (u) => {
      await page.evaluate((x) => { gotoPage(S.objPage.get(x), { focus: x }); select(x); }, u);
      await page.waitForFunction((x) => S.sel === x, u);
    };
    // The module's own expression: its two blocks folded into one, titled with the expression.
    await page.evaluate((p) => gotoPage(p), AUSDRUCK);
    await page.waitForSelector('.expr-fold[data-expr="markise_i2__"]', { timeout: 10000 });
    assert.equal(await page.getAttribute('.expr-fold[data-expr="markise_i2__"]', "aria-label"), "markise.In2 ← licht.Q and teiler.AQ >= 28");
    for (const u of [EXA, EXG]) assert.ok(await page.$eval(`.blk[data-uuid="${u}"]`, (n) => n.classList.contains("expr-hidden")));
    // A click opens it and selects its top block: the expression, what it feeds and its line, read-only.
    await page.click('.expr-fold[data-expr="markise_i2__"]');
    await page.waitForFunction((u) => S.sel === u && document.querySelector('.expr-grp[data-expr="markise_i2__"]'), EXA);
    assert.ok(!(await page.$eval(`.blk[data-uuid="${EXG}"]`, (n) => n.classList.contains("expr-hidden"))));
    await waitText(ex, /Expression\s*licht\.Q and teiler\.AQ >= 28/);
    await waitText(ex, /Feeds\s*Markise\.In2/);
    await waitText(ex, /pages\/ausdruck\.loxl:5/);
    await waitText(ex, /Written in the module's own file: change it there/);
    assert.equal(await page.$(`${ex} [data-ex="save"]`), null);
    // Its title folds it again.
    await page.click('text.expr-t[data-expr="markise_i2__"]');
    await page.waitForSelector('.expr-fold[data-expr="markise_i2__"]');
    // An expression of vislox's on Pumpe's free input: `vx_<pumpe>.In1 <- …` in vislox's file.
    const div = `vx_${DIV.replace(/-/g, "_")}`, pumpe = `vx_${PUMPE.replace(/-/g, "_")}`;
    await go(PUMPE);
    await page.click(`${ex} .ex-new summary`);
    await page.waitForSelector(`${ex} [data-ex="add"]`);
    await page.selectOption(`${ex} [data-ex-field="new:port"]`, "I1");
    await page.fill(`${ex} [data-ex-field="new:text"]`, `${div}.AQ >= 30`);
    let ops = await send(() => page.click(`${ex} [data-ex="add"]`), "add an expression");
    assert.deepEqual(ops, [{ op: "expr", uuid: PUMPE, port: "I1", value: `${div}.AQ >= 30` }]);
    assert.ok(claims().includes(`${pumpe}.In1 <- ${div}.AQ >= 30\n`), claims());
    const id = `${pumpe}_i1__`;
    await page.waitForFunction((i) => S.edits?.expressions?.some((e) => e.id === i && e.ours && e.pending), id, { timeout: 10000 });
    // Its top block is selected, and the expression is edited there.
    await page.waitForFunction((i) => S.edits.expressions.find((e) => e.id === i).blocks[0] === S.sel, id, { timeout: 10000 });
    await waitText(ex, /Not deployed: new since the deployed config/);
    await page.fill(`${ex} [data-ex-field="${id}:text"]`, `${div}.AQ >= 30 and ${div}.AQ < 40`);
    ops = await send(() => page.press(`${ex} [data-ex-field="${id}:text"]`, "Enter"), "change the expression");
    assert.deepEqual(ops, [{ op: "expr", uuid: PUMPE, port: "I1", value: `${div}.AQ >= 30 and ${div}.AQ < 40` }]);
    await page.waitForFunction((i) => S.edits?.expressions?.find((e) => e.id === i)?.top_type === "And", id, { timeout: 10000 });
    // The Changes list reads it as the expression, not as blocks added and removed.
    await waitText("#changes", new RegExp(`expression on ${pumpe}\\.In1: ${div}\\.AQ >= 30 and ${div}\\.AQ < 40 \\(new\\)`));
    // Name this expression: a signal of the page, and the input fed from it, in one request.
    await page.waitForFunction((i) => S.edits.expressions.find((e) => e.id === i).blocks[0] === S.sel, id, { timeout: 10000 });
    await page.fill(`${ex} [data-ex-field="${id}:name"]`, "vx_pumpe_an");
    ops = await send(() => page.click(`${ex} [data-ex="name"]`), "name the expression");
    assert.deepEqual(ops, [
      { op: "signal", slug: "vx_pumpe_an", page: AUSDRUCK, value: `${div}.AQ >= 30 and ${div}.AQ < 40` },
      { op: "expr", uuid: PUMPE, port: "I1", value: "vx_pumpe_an" },
    ]);
    assert.ok(claims().includes(`${pumpe}.In1 <- vx_pumpe_an.Q\n`), claims());
    assert.match(readFileSync(join(dir, "pages", "vislox", "ausdruck.loxl"), "utf8"), /^vx_pumpe_an = vx_\w+\.AQ >= 30 and vx_\w+\.AQ < 40$/m);
    await page.waitForFunction(() => S.edits?.expressions?.some((e) => e.kind === "signal" && e.sink === "vx_pumpe_an" && e.ours) &&
      !S.edits.expressions.some((e) => e.id.startsWith("vx_80000004")), null, { timeout: 10000 });
    // Revert drops the signal made here and gives the input it feeds back what the deployed files fed it (nothing),
    // in one request; undone, it is named again.
    await page.waitForFunction(() => S.sel === S.edits.expressions.find((e) => e.sink === "vx_pumpe_an").blocks[0], null, { timeout: 10000 });
    ops = await send(() => page.click(`${ex} [data-ex="revert"][data-id="vx_pumpe_an__"]`), "revert the signal");
    assert.deepEqual(ops, [
      { op: "expr", uuid: PUMPE, port: "I1", value: null },
      { op: "signal", slug: "vx_pumpe_an", page: AUSDRUCK, value: null },
    ]);
    await page.waitForFunction(() => S.edits && !S.edits.expressions.some((e) => e.sink === "vx_pumpe_an" || e.id.startsWith("vx_80000004")), null, { timeout: 10000 });
    assert.ok(!claims().includes(pumpe), claims());
    await undoKey(false, () => S.edits?.expressions?.some((e) => e.sink === "vx_pumpe_an"));
    assert.ok(claims().includes(`${pumpe}.In1 <- vx_pumpe_an.Q\n`), claims());
    await go(await page.evaluate(() => S.edits.expressions.find((e) => e.sink === "vx_pumpe_an").blocks[0]));
    // The signal's label.
    await page.waitForFunction(() => S.sel === S.edits.expressions.find((e) => e.sink === "vx_pumpe_an").blocks[0], null, { timeout: 10000 });
    await waitText(ex, /Signal\s*vx_\w+\.AQ >= 30/);
    await page.fill(`${ex} [data-ex-field="vx_pumpe_an__:title"]`, "Pumpe an");
    ops = await send(() => page.click(`${ex} [data-ex="save"]`), "label the signal");
    assert.deepEqual(ops, [{ op: "signal", slug: "vx_pumpe_an", page: AUSDRUCK, value: `${div}.AQ >= 30 and ${div}.AQ < 40`, title: "Pumpe an" }]);
    await page.waitForFunction(() => S.edits?.expressions?.find((e) => e.sink === "vx_pumpe_an")?.title === "Pumpe an", null, { timeout: 10000 });
    // Undone: the label, then the naming (the expression back on the input, the signal dropped).
    ops = await undoKey(false, () => S.edits?.expressions?.find((e) => e.sink === "vx_pumpe_an")?.title === undefined);
    assert.deepEqual(ops, [{ op: "signal", slug: "vx_pumpe_an", page: AUSDRUCK, value: `${div}.AQ >= 30 and ${div}.AQ < 40` }]);
    ops = await undoKey(false, (i) => S.edits?.expressions?.some((e) => e.id === i) && !S.edits.expressions.some((e) => e.sink === "vx_pumpe_an"), id);
    assert.deepEqual(ops, [
      { op: "expr", uuid: PUMPE, port: "I1", value: `${div}.AQ >= 30 and ${div}.AQ < 40` },
      { op: "signal", slug: "vx_pumpe_an", page: AUSDRUCK, value: null },
    ]);
    assert.ok(claims().includes(`${pumpe}.In1 <- ${div}.AQ >= 30 and ${div}.AQ < 40\n`), claims());
    // Undone to the start: nothing of vislox's feeds Pumpe.
    await undoKey(false, (i) => S.edits?.expressions?.find((e) => e.id === i)?.top_type === "GreaterEqual", id);
    await undoKey(false, (i) => !S.edits?.expressions?.some((e) => e.id === i), id);
    assert.ok(!claims().includes(pumpe), claims());
  });
  await step("template instances are framed, named in Properties, added from lox's library, edited and removed (#83)", async () => {
    await settled();
    const tp = '#props [data-body="Template"]';
    const send = async (act, what) => {
      const n = edits.length;
      await act();
      await until(() => edits.length > n && statuses[n] !== undefined, what);
      assert.equal(statuses[n], 200, what);
      await settled();
      return edits[n].ops;
    };
    const vfile = (f) => { const p = join(dir, "pages", "vislox", f); return existsSync(p) ? readFileSync(p, "utf8") : ""; };
    // The module's own instance: its block framed, the frame headed by the call.
    await page.evaluate((p) => gotoPage(p), VORLAGEN);
    await page.waitForSelector('.tpl-head[data-inst="nord"]', { timeout: 10000 });
    assert.equal(await page.getAttribute('.tpl-head[data-inst="nord"]', "aria-label"), 'nord: Paar "Nord"');
    assert.match(await page.textContent('.tpl-head[data-inst="nord"]'), /quelle: teiler\.AQ\s+→ Q/);
    // Fitted to its blocks, the page may draw the header under the toolbar: clicked where it is.
    await page.dispatchEvent('.tpl-head[data-inst="nord"] .tpl-t', "click");
    await page.waitForFunction((u) => S.sel === u, NORD_G);
    await waitText(tp, /Block\s*nord\.g/);
    await waitText(tp, /Template\s*Paar\s*pages\/vorlage\.loxl:2/);
    await waitText(tp, /quelle\s*teiler\.AQ/);
    await waitText(tp, /pages\/vorlage\.loxl:6/);
    await waitText(tp, /Written in the module's own file: change it there/);
    assert.equal(await page.$(`${tp} [data-tp="save"]`), null);
    // Add block offers lox's library's templates: an instance of Treppenlicht, named for it.
    const vi = `vx_${"10000001-0000-0010-ffff504f94000001".replace(/-/g, "_")}`;
    await page.click("#btn-add-block");
    await page.waitForSelector("#ab-q");
    await page.fill("#ab-q", "Treppenlicht");
    assert.match(await page.textContent("#ab-list [aria-selected=true]"), /Treppenlicht[\s\S]*Template/);
    await page.press("#ab-q", "Enter");
    await page.waitForSelector('#ab-body [data-ab-field="slug"]');
    assert.equal(await page.inputValue('#ab-body [data-ab-field="slug"]'), "vx_treppenlicht_1");
    // Its object parameter is required.
    await page.click('#ab-body [data-ab="add"]');
    await page.waitForSelector('#ab-body [data-ab-field="a:taster"][aria-invalid="true"]');
    await page.fill('#ab-body [data-ab-field="a:taster"]', vi);
    await page.fill('#ab-body [data-ab-field="a:titel"]', "Flur");
    let ops = await send(() => page.press('#ab-body [data-ab-field="a:titel"]', "Enter"), "add an instance");
    assert.deepEqual(ops, [{ op: "instance", slug: "vx_treppenlicht_1", page: VORLAGEN, value: { template: "Treppenlicht", args: { taster: vi, titel: "Flur" } } }]);
    assert.match(vfile("vorlagen.loxl"), /# Template instances[^\n]*\nvx_treppenlicht_1 = Treppenlicht\(taster: vx_10000001_0000_0010_ffff504f94000001, titel: "Flur"\)\n/);
    assert.match(vfile("templates.loxl"), /^template Treppenlicht\(/m);
    // Its block is selected, framed, and named in Properties; the instance is edited there.
    await page.waitForFunction(() => { const x = S.edits?.templates?.instances?.find((i) => i.path === "vx_treppenlicht_1"); return x?.ours && x.blocks.length && ciEq(S.sel, x.blocks[0].uuid); }, null, { timeout: 10000 });
    await page.waitForSelector('.tpl-head[data-inst="vx_treppenlicht_1"]');
    await waitText(tp, /Block\s*vx_treppenlicht_1\.ausgang/);
    await waitText(tp, /lox's library/);
    await waitText(tp, /Not deployed: vx_treppenlicht_1 is new/);
    await page.fill(`${tp} [data-tp-field="e:a:dauer"]`, "5min");
    ops = await send(() => page.press(`${tp} [data-tp-field="e:a:dauer"]`, "Enter"), "change an argument");
    assert.deepEqual(ops, [{ op: "instance", slug: "vx_treppenlicht_1", page: VORLAGEN, value: { template: "Treppenlicht", args: { taster: vi, dauer: "5min", titel: '"Flur"' } } }]);
    assert.match(vfile("vorlagen.loxl"), /vx_treppenlicht_1 = Treppenlicht\(taster: \w+, dauer: 5min, titel: "Flur"\)/);
    // A value of the wrong kind: lox's words, nothing changed.
    await page.waitForFunction(() => document.querySelector('#props [data-tp-field="e:a:dauer"]'));
    await page.fill(`${tp} [data-tp-field="e:a:dauer"]`, "5");
    await page.press(`${tp} [data-tp-field="e:a:dauer"]`, "Enter");
    await waitText(tp, /dauer is a duration parameter/);
    assert.match(vfile("vorlagen.loxl"), /dauer: 5min/);
    // The Changes list puts its changes under the instance.
    await waitText("#changes", /vx_treppenlicht_1: Treppenlicht \(new\)/);
    // Not deployed: it can be removed; undone, it is back.
    await page.waitForFunction(() => document.querySelector('#props [data-tp="drop"]'));
    ops = await send(() => page.click(`${tp} [data-tp="drop"]`), "remove the instance");
    assert.deepEqual(ops, [{ op: "instance", slug: "vx_treppenlicht_1", page: VORLAGEN, value: null }]);
    await page.waitForFunction(() => S.edits && !S.edits.templates.instances.some((i) => i.path === "vx_treppenlicht_1"), null, { timeout: 10000 });
    assert.doesNotMatch(vfile("vorlagen.loxl"), /vx_treppenlicht_1 =/);
    assert.equal(vfile("templates.loxl"), "", "the library's copy goes with its last instance");
    await undoKey(false, () => S.edits?.templates?.instances?.some((i) => i.path === "vx_treppenlicht_1"));
    assert.match(vfile("vorlagen.loxl"), /dauer: 5min/);
    await undoKey(false, () => S.edits?.templates?.instances?.some((i) => i.path === "vx_treppenlicht_1" && !i.args.some(([k]) => k === "dauer")));
    await undoKey(false, () => S.edits && !S.edits.templates.instances.some((i) => i.path === "vx_treppenlicht_1"));
    assert.equal(vfile("templates.loxl"), "");
  });
  await step("virtual inputs and Modbus devices are added from the project tree, edited and taken out (#84)", async () => {
    await settled();
    const send = async (act, what) => {
      const n = edits.length;
      await act();
      await until(() => edits.length > n && statuses[n] !== undefined, what);
      assert.equal(statuses[n], 200, what);
      await settled();
      return edits[n].ops;
    };
    const pfile = () => { const p = join(dir, "pages", "vislox", "_periphery.loxl"); return existsSync(p) ? readFileSync(p, "utf8") : ""; };
    const claims = () => readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    const row = (u) => `#tree .tn-row[data-uuid="${u}"]`;
    // The Virtual Inputs folder offers Add; a page does not.
    await page.evaluate((u) => revealInTree(u), VI_F);
    await page.waitForSelector(`${row(VI_F)} [data-add-under]:not([disabled])`);
    assert.equal(await page.$(`${row(OG)} [data-add-under]`), null);
    await page.click(`${row(VI_F)} [data-add-under]`);
    await page.waitForSelector("#ab-q");
    assert.equal(await page.textContent("#ab-h"), "Add an object");
    assert.deepEqual(await page.$$eval("#ab-list [role=option] .ac", (n) => n.map((x) => x.textContent).sort()), ["VirtualHttpIn", "VirtualIn", "VirtualTextIn", "VirtualUdpIn"]);
    await page.fill("#ab-q", "VirtualHttpIn");
    await page.press("#ab-q", "Enter");
    await page.waitForSelector('#ab-body [data-ab-field="s:Address"]');
    // Address and Poll come first: lox makes none without them, and the browser says so before it sends anything.
    assert.deepEqual((await page.$$eval("#ab-body [data-ab-field]", (n) => n.map((x) => x.dataset.abField))).slice(0, 3), ["title", "s:Address", "s:Poll"]);
    await page.fill('#ab-body [data-ab-field="title"]', "Zähler");
    await page.fill('#ab-body [data-ab-field="s:Address"]', "http://192.0.2.10/status");
    const n0 = edits.length;
    await page.click('#ab-body [data-ab="add"]');
    await page.waitForSelector('#ab-body [data-ab-field="s:Poll"][aria-invalid="true"]');
    assert.equal(edits.length, n0, "nothing sent without Poll");
    await page.fill('#ab-body [data-ab-field="s:Poll"]', "1min");
    let ops = await send(() => page.click('#ab-body [data-ab="add"]'), "add a virtual HTTP input");
    assert.deepEqual(ops, [{ op: "add_block", page: VI_F, type: "VirtualHttpIn", title: "Zähler", params: {}, settings: { Address: "http://192.0.2.10/status", Poll: "1min" } }]);
    assert.match(pfile(), /^vx_virtualhttpin_1 = VirtualHttpIn\("Zähler", Address: "http:\/\/192\.0\.2\.10\/status", Poll: 1min\)$/m);
    assert.doesNotMatch(pfile(), /^page /m);
    // It is selected, in the tree below the folder, added here and not deployed.
    const http = await (await page.waitForFunction(() => S.edits?.blocks?.find((b) => b.slug === "vx_virtualhttpin_1")?.uuid, null, { timeout: 20000 })).jsonValue();
    await page.waitForFunction((u) => ciEq(S.sel, u), http);
    await page.waitForSelector(`${row(http)}`);
    await waitText('[data-body="Edit"]', /added here[\s\S]*not deployed/);
    // Its command: the device offers only what hangs below it.
    await page.click(`${row(http)} [data-add-under]`);
    await page.waitForSelector("#ab-q");
    assert.deepEqual(await page.$$eval("#ab-list [role=option] .ac", (n) => n.map((x) => x.textContent)), ["VirtualHttpInCmd"]);
    await page.press("#ab-q", "Enter");
    await page.fill('#ab-body [data-ab-field="title"]', "Leistung");
    await page.fill('#ab-body [data-ab-field="s:Check"]', '"power":\\v');
    // The History is held (until `release`) after the command's add: the page still shows the config before it
    // while the next edit is started. That edit is refused in the browser, nothing sent for the server to 409.
    let release;
    const held = new Promise((r) => { release = r; });
    await page.route("**/api/history", async (r) => { await held; await r.continue().catch(() => {}); });
    const nc = edits.length;
    try {
      await page.click('#ab-body [data-ab="add"]');
      await until(() => edits.length > nc && statuses[nc] !== undefined, "add its command");
      assert.equal(statuses[nc], 200);
      // Once its reply is handled (not when its headers are in): the page is waiting for the History.
      await page.waitForFunction(() => showingWhy(), null, { timeout: 10000 });
      assert.match(await page.evaluate(() => editAct(editRev().blob, [{ op: "title", uuid: S.sel, value: "x" }]).then(() => "sent", (e) => e.message)), /still being shown/);
      assert.equal(edits.length, nc + 1, "nothing was sent");
    } finally {
      release();
      await page.unroute("**/api/history");
    }
    await settled();
    ops = edits[nc].ops;
    assert.deepEqual(ops, [{ op: "add_block", page: http, type: "VirtualHttpInCmd", title: "Leistung", params: {}, settings: { Check: '"power":\\v' } }]);
    assert.match(pfile(), /^vx_virtualhttpincmd_1 = VirtualHttpInCmd\("Leistung", device: vx_virtualhttpin_1, Check: "\\"power\\":\\\\v"\)$/m);
    // A device of Loxone Config's: the Modbus server takes a Modbus device, through vislox's extern of it.
    await page.evaluate((u) => revealInTree(u), WP);
    await page.waitForSelector(`${row(WP)} [data-add-under]:not([disabled])`);
    await page.click(`${row(WP)} [data-add-under]`);
    await page.waitForSelector("#ab-q");
    await page.press("#ab-q", "Enter");
    await page.fill('#ab-body [data-ab-field="title"]', "Regler");
    ops = await send(() => page.press('#ab-body [data-ab-field="title"]', "Enter"), "add a Modbus device");
    assert.deepEqual(ops, [{ op: "add_block", page: WP, type: "ModbusDev", title: "Regler", params: {}, settings: {} }]);
    const wp = `vx_${WP.replace(/-/g, "_")}`;
    assert.match(pfile(), new RegExp(`^vx_modbusdev_1 = ModbusDev\\("Regler", device: ${wp}\\)$`, "m"));
    assert.ok(claims().includes(`extern ${wp} = ModbusServer(uuid: "${WP}")`), claims());
    // The Changes list names what was added.
    await waitText("#changes", /Zähler/);
    await waitText("#changes", /Leistung/);
    // A setting changed in Properties; a required one cannot go, and the server says so.
    await page.evaluate((u) => { select(u); revealInTree(u); }, http);
    await page.waitForFunction((u) => ciEq(S.sel, u) && document.querySelector('#props [data-ed="open"]'), http);
    await page.click('#props [data-ed="open"]');
    assert.equal(await page.inputValue(field("s:Poll")), "1min");
    await page.fill(field("s:Poll"), "120");
    ops = await send(() => page.click('#props [data-ed="save"]'), "change the polling cycle");
    assert.deepEqual(ops, [{ op: "setting", uuid: http, name: "Poll", value: "120" }]);
    assert.match(pfile(), /Poll: 120\)$/m);
    await page.waitForFunction(() => document.querySelector('#props [data-ed="open"]'));
    await page.click('#props [data-ed="open"]');
    await page.fill(field("s:Address"), "");
    const n1 = edits.length;
    await page.click('#props [data-ed="save"]');
    await until(() => statuses[n1] !== undefined, "the refused setting");
    assert.equal(statuses[n1], 422);
    await waitText("#props", /needs Address/);
    assert.match(pfile(), /Address: "http:\/\/192\.0\.2\.10\/status"/);
    await page.click('#props [data-ed="cancel"]');
    // The device cannot go while its command hangs below it: Remove is off, with the reason.
    await page.waitForSelector('#props [data-ed="remove"][disabled]');
    assert.match(await page.getAttribute('#props [data-ed="remove"]', "title"), /Leistung hangs below it/);
    // Undone in turn: the setting, the Modbus device (and the extern with it), the command, the input.
    await undoKey(false, () => S.edits?.blocks.find((b) => b.slug === "vx_virtualhttpin_1")?.settings.find((x) => x.name === "Poll")?.value === "1min");
    assert.match(pfile(), /Poll: 1min\)$/m);
    await undoKey(false, () => !S.edits?.blocks.some((b) => b.slug === "vx_modbusdev_1"));
    assert.ok(!claims().includes(wp), claims());
    await undoKey(false, () => !S.edits?.blocks.some((b) => b.slug === "vx_virtualhttpincmd_1"));
    await undoKey(false, () => !S.edits?.blocks.some((b) => b.slug === "vx_virtualhttpin_1"));
    assert.doesNotMatch(pfile(), /= Virtual/);
  });
  await step("statistics are switched on a Tree input, and a Tree device is created unpaired and taken out (#85)", async () => {
    await settled();
    const send = async (act, what) => {
      const n = edits.length;
      await act();
      await until(() => edits.length > n && statuses[n] !== undefined, what);
      assert.equal(statuses[n], 200, what);
      await settled();
      return edits[n].ops;
    };
    const hfile = () => { const p = join(dir, "pages", "vislox", "_hardware.loxl"); return existsSync(p) ? readFileSync(p, "utf8") : ""; };
    const row = (u) => `#tree .tn-row[data-uuid="${u}"]`;
    const hwSlug = (u) => `vx_hw_${u.replace(/-/g, "_")}`;
    // The temperature records nothing; the touch zone does, read-only on any revision.
    await page.evaluate((u) => { select(u); revealInTree(u); }, HW_S);
    await waitText("#props", /Statistics\s*on/);
    await page.evaluate((u) => { select(u); revealInTree(u); }, HW_A);
    await waitText("#props", /Statistics\s*off/);
    await page.waitForSelector('#props [data-ed="open"]');
    await page.click('#props [data-ed="open"]');
    assert.equal(await page.inputValue(field("statistics")), "off");
    await page.selectOption(field("statistics"), "on");
    let ops = await send(() => page.click('#props [data-ed="save"]'), "switch statistics on");
    assert.deepEqual(ops, [{ op: "statistics", uuid: HW_A, value: "on" }]);
    assert.match(hfile(), new RegExp(`^${hwSlug(HW_D)} = TreeDevice\\(uuid: "${HW_D}"\\)$`, "m"));
    assert.match(hfile(), new RegExp(`^${hwSlug(HW_A)} = TreeAsensor\\(device: ${hwSlug(HW_D)}, uuid: "${HW_A}", Statistics: on\\)$`, "m"));
    // Pending: on in the config, not deployed, with Revert; the Changes list names it.
    await waitText("#props", /Statistics\s*off\s*on/);
    await page.waitForSelector('#props [data-ed="revert"]');
    await waitText("#changes", /Temperatur Bad/);
    ops = await send(() => page.click('#props [data-ed="revert"]'), "revert the statistics");
    assert.deepEqual(ops, [{ op: "statistics", uuid: HW_A, value: null }]);
    assert.equal(hfile(), "");
    // The Tree Extension offers Add, for a Tree device of the kinds lox creates.
    await page.evaluate((u) => revealInTree(u), HW_T);
    await page.waitForSelector(`${row(HW_T)} [data-add-under]:not([disabled])`);
    await page.click(`${row(HW_T)} [data-add-under]`);
    await page.waitForSelector("#ab-q");
    assert.deepEqual(await page.$$eval("#ab-list [role=option] .ac", (n) => n.map((x) => x.textContent)), ["TreeDevice"]);
    await page.press("#ab-q", "Enter");
    await page.waitForSelector('#ab-body [data-ab-field="s:kind"]');
    assert.deepEqual(await page.$$eval('#ab-body [data-ab-field="s:kind"] option', (n) => n.map((x) => x.value).filter(Boolean)), ["TouchTree", "PresenceSensorTree", "CorridorLightTree", "ValveTree"]);
    await page.fill('#ab-body [data-ab-field="title"]', "Melder Flur");
    await page.selectOption('#ab-body [data-ab-field="s:kind"]', "PresenceSensorTree");
    await page.selectOption('#ab-body [data-ab-field="s:branch"]', "left");
    ops = await send(() => page.click('#ab-body [data-ab="add"]'), "add a Tree device");
    assert.deepEqual(ops, [{ op: "add_block", page: HW_T, type: "TreeDevice", title: "Melder Flur", params: {}, settings: { kind: "PresenceSensorTree", branch: "left" } }]);
    assert.match(hfile(), new RegExp(`^extern ${hwSlug(HW_T)} = LoxTree\\(uuid: "${HW_T}"\\)$`, "m"));
    assert.match(hfile(), new RegExp(`^vx_presencesensortree_1 = TreeDevice\\("Melder Flur", kind: "PresenceSensorTree", tree: ${hwSlug(HW_T)}, branch: "left"\\)$`, "m"));
    // Selected, marked unpaired in the tree and in Properties, added here and not deployed.
    const dev = await (await page.waitForFunction(() => S.edits?.blocks?.find((b) => b.slug === "vx_presencesensortree_1")?.uuid, null, { timeout: 20000 })).jsonValue();
    await page.waitForFunction((u) => ciEq(S.sel, u), dev);
    await page.waitForSelector(`${row(dev)} .tn-tag`);
    assert.equal(await page.textContent(`${row(dev)} .tn-tag`), "unpaired");
    await waitText("#props", /Pairing\s*Created here, not paired/);
    await waitText('[data-body="Edit"]', /added here[\s\S]*not deployed/);
    assert.equal(await page.$('#props [data-ed="rename-open"]'), null);
    // Its input's statistics: by its designation, the only value it takes before Apply.
    const i1 = dev.slice(0, 9) + "0000-0001-ffff504f94000001";
    await page.evaluate((u) => { select(u); revealInTree(u); }, i1);
    await page.waitForSelector('#props [data-ed="open"]');
    await page.click('#props [data-ed="open"]');
    assert.deepEqual(await page.$$eval("#props [data-ed-field]", (n) => n.map((x) => x.dataset.edField)), ["statistics"]);
    await page.selectOption(field("statistics"), "on");
    ops = await send(() => page.click('#props [data-ed="save"]'), "statistics of the new device's input");
    assert.deepEqual(ops, [{ op: "statistics", uuid: i1, value: "on" }]);
    assert.match(hfile(), /^vx_presencesensortree_1_i1 = TreeSensor\(device: vx_presencesensortree_1, iname: "I1", Statistics: on\)$/m);
    // Taken out: \`removed\`, since lox keeps it from its first compile on; undone, it comes back as it was.
    await page.evaluate((u) => { select(u); revealInTree(u); }, dev);
    await page.waitForSelector('#props [data-ed="remove"]:not([disabled])');
    ops = await send(() => page.click('#props [data-ed="remove"]'), "take the device out");
    assert.deepEqual(ops, [{ op: "remove_block", uuid: dev }]);
    assert.doesNotMatch(hfile(), /= TreeDevice\(/);
    assert.match(hfile(), /^removed vx_presencesensortree_1$/m);
    await undoKey(false, () => S.edits?.blocks.some((b) => b.hardware && b.uuid));
    assert.match(hfile(), /= TreeDevice\("Melder Flur", kind: "PresenceSensorTree"/);
    assert.match(hfile(), /_i1 = TreeSensor\(device: vx_presencesensortree_\d, iname: "I1", Statistics: on\)$/m);
    // Taken out again, for the steps after this one.
    const back = await page.evaluate(() => S.edits.blocks.find((b) => b.hardware && b.uuid).uuid);
    await page.evaluate((u) => { select(u); revealInTree(u); }, back);
    await page.waitForSelector('#props [data-ed="remove"]:not([disabled])');
    await send(() => page.click('#props [data-ed="remove"]'), "take the device out again");
    assert.doesNotMatch(hfile(), /= TreeDevice\(/);
  });
  await step("device Revert and Undo restore the whole adoption group, root first (#133)", async () => {
    await settled();
    // The reply comes after the History and the edits have shown the adoption (held a while): the Changes list
    // draws its rows while the edit is still running, and Revert must come on when it ends.
    await page.route("**/api/edit", async (r) => { const resp = await r.fetch(); await new Promise((x) => setTimeout(x, 2500)); await r.fulfill({ response: resp }).catch(() => {}); }, { times: 1 });
    const reply = await page.evaluate(([u, slug]) => adoptSend(u, [{ op: "adopt", blocks: [{ uuid: u.toUpperCase(), slug }] }]), [IMPORT_HTTP, "vx_imported"]);
    assert.ok(reply, await page.evaluate(() => JSON.stringify({err: AD.err, state: adoptState()})));
    assert.equal(reply.adopted[0].uuid, IMPORT_HTTP);
    assert.equal(reply.adopted.length, 2);
    await page.waitForFunction(([p,c]) => [p,c].every((u) => S.edits?.blocks?.some((b) => b.uuid === u && b.adopted)), [IMPORT_HTTP, IMPORT_CMD]);
    await settled();
    await page.waitForSelector('#changes .adoption:has-text("Imported HTTP") button:has-text("Revert"):not([disabled])', { timeout: 10000 });
    await page.click('#changes .adoption:has-text("Imported HTTP") button:has-text("Revert")');
    await page.waitForFunction(([p,c]) => !S.edits?.blocks?.some((b) => [p,c].includes(b.uuid)), [IMPORT_HTTP, IMPORT_CMD]);
    await settled();
    await undoKey(false, ([p,c]) => [p,c].every((u) => S.edits?.blocks?.some((b) => b.uuid === u && b.adopted)), [IMPORT_HTTP, IMPORT_CMD]);
    await undoKey(false, ([p,c]) => !S.edits?.blocks?.some((b) => [p,c].includes(b.uuid)), [IMPORT_HTTP, IMPORT_CMD]);
  });
  await step("calendar references follow mode adoption in either order (#133)", async () => {
    const silvester = "10000017-0000-0170-ffff504f94000001";
    for (const entryFirst of [true, false]) {
      for (const [u,slug] of entryFirst ? [[silvester,"vx_party_day"],[PARTY,"vx_party"]] : [[PARTY,"vx_party"],[silvester,"vx_party_day"]]) {
        await settled();
        const reply = await page.evaluate(([uuid,slug]) => adoptSend(uuid, [{op:"adopt",blocks:[{uuid,slug}]}]), [u,slug]);
        assert.ok(reply, "adoption compiled");
        await page.waitForFunction((u) => S.edits?.blocks?.some((b) => b.uuid === u && b.adopted), u);
      }
      await settled();
      const pfile = join(dir,"pages","vislox","_periphery.loxl");
      assert.match(readFileSync(pfile,"utf8"), /switches: vx_party, every: 12-31/);
      await page.evaluate((u) => adoptHandBack(u), PARTY);
      await page.waitForFunction((u) => !S.edits?.blocks?.some((b) => b.uuid === u), PARTY);
      await settled();
      assert.match(readFileSync(pfile,"utf8"), /switches: "Party", every: 12-31/);
      await undoKey(false, (u) => S.edits?.blocks?.some((b) => b.uuid === u && b.adopted), PARTY);
      await page.evaluate((uuids) => adoptSend(uuids[0], [{op:"unadopt",uuids}]), [PARTY,silvester]);
      await page.waitForFunction((uuids) => !S.edits?.blocks?.some((b) => uuids.includes(b.uuid)), [PARTY,silvester]);
      await settled();
    }
  });
  await step("object-tree adoption, edit, undo and hand back (#86)", async () => {
    for (const [uuid, slug] of [[PARTY, "vx_party"], [HEILIGABEND, "vx_christmas"], ["10000001-0000-0010-ffff504f94000001", "vx_temperature"]]) {
      await settled();
      await page.evaluate((u) => select(u), uuid);
      await page.waitForSelector('#props [data-ed="adopt-open"]:not([disabled])');
      await page.click('#props [data-ed="adopt-open"]');
      await page.fill("#ad-slug", slug);
      await page.click('#props [data-ed="adopt"]');
      await page.waitForFunction((u) => S.edits?.blocks?.some((b) => b.uuid === u && b.adopted), uuid, { timeout: 20000 });
      await settled();
      await page.waitForSelector("#changes .adoption");
      await page.click('#props [data-ed="open"]');
      await page.fill('#props [data-ed-field="title"]', "Changed title");
      await page.click('#props [data-ed="save"]');
      await page.waitForFunction((u) => S.view?.objects[u]?.title === "Changed title", uuid, { timeout: 20000 });
      await undoKey(false, () => !Object.values(S.view.objects).some((o) => o.title === "Changed title"));
      await undoKey(false, () => !S.edits?.blocks?.some((b) => b.adopted));
      await settled();
      assert.equal(JSON.parse(readFileSync(join(dir, "logic.lock.json"), "utf8")).objects[slug], undefined);
    }
  });

  await step("operating modes are added, and calendar entries added, picked on the year, edited and undone (#86)", async () => {
    await settled();
    const send = async (act, what, status = 200) => {
      const n = edits.length;
      await act();
      await until(() => edits.length > n && statuses[n] !== undefined, what);
      assert.equal(statuses[n], status, what);
      await settled();
      return edits[n].ops;
    };
    const pfile = () => { const p = join(dir, "pages", "vislox", "_periphery.loxl"); return existsSync(p) ? readFileSync(p, "utf8") : ""; };
    const row = (u) => `#tree .tn-row[data-uuid="${u}"]`;
    // The calendar folder's Add in the project tree opens the entry form of the Operating modes view.
    await page.evaluate((u) => revealInTree(u), CAL_F);
    await page.waitForSelector(`${row(CAL_F)} [data-add-under]:not([disabled])`);
    await page.click(`${row(CAL_F)} [data-add-under]`);
    await page.waitForFunction(() => MD.open && MF.open && MF.what === "entry");
    await waitText("#md-form", /Add a calendar entry/);
    await page.click('#md-form [data-mf-act="cancel"]');
    await page.waitForFunction(() => !MF.open);
    // An entry Loxone Config made is not edited here, and says why.
    await page.waitForSelector(`[data-md-edit="${HEILIGABEND}"][disabled]`);
    assert.match(await page.getAttribute(`[data-md-edit="${HEILIGABEND}"]`, "title"), /Loxone Config made this entry/);
    // Add mode: only its name; lox numbers it.
    await page.click("#md-add-mode:not([disabled])");
    await page.fill("#mf-title", "Grill");
    let ops = await send(() => page.click('#md-form [data-mf-act="save"]'), "add a mode");
    assert.deepEqual(ops, [{ op: "add_block", page: MODE_F, type: "Mode", title: "Grill", params: {}, settings: {} }]);
    assert.match(pfile(), /^vx_mode_1 = Mode\("Grill"\)$/m);
    await waitText("#md-legend", /Grill/);
    await waitText("#md-form", /Added and compiled: pending, not deployed/);
    const grill = await page.evaluate(() => S.view.modes.find((m) => m.title === "Grill").uuid);
    // Add entry: the mode picked, the days dragged on the year (4 to 5 July 2026), outlined while drafted.
    await page.evaluate(() => { MD.year = 2026; renderModes(); });
    await page.click("#md-add-entry:not([disabled])");
    await page.fill("#mf-title", "Grillfest");
    await page.keyboard.press("Tab");
    await page.keyboard.press("g");
    assert.equal(await page.inputValue("#mf-mode"), grill);
    const [d4, d5] = await page.evaluate(() => [mdDay(2026, 7, 4), mdDay(2026, 7, 5)]);
    // Keyboard-only date selection: Shift+Tab to the grid, Enter picks a day,
    // and Shift+Enter extends the period from that first day.
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Shift+Tab");
    const anchor = await page.evaluate(() => +document.activeElement.dataset.day);
    assert.ok(anchor, "Tab reaches the calendar day control");
    await page.keyboard.press("Enter");
    await page.waitForFunction((n) => {
      const d = MF.draft, ymd = mdYmdText(n), md = mdMdText(n), [y, m, day] = mdYmd(n);
      return d.kind === "period" ? d.from === ymd && d.to === ymd
        : d.kind === "yearly" ? d.yfrom === md && d.yto === md
          : d.kind === "every" ? d.every === md
            : d.kind === "easter" && d.easter === String(n - mdDay(y, ...easterSunday(y)));
    }, anchor);
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("Shift+Enter");
    await page.waitForFunction((n) => {
      const d = MF.draft, a = mdYmdText(n), b = mdYmdText(n + 1), am = mdMdText(n), bm = mdMdText(n + 1);
      return d.kind === "yearly" ? d.yfrom === am && d.yto === bm : d.kind === "period" && d.from === a && d.to === b;
    }, anchor);
    await page.hover(`#md-grid [data-day="${d4}"]`);
    await page.mouse.down();
    await page.hover(`#md-grid [data-day="${d5}"]`);
    await page.mouse.up();
    assert.equal(await page.inputValue("#mf-kind"), "period");
    assert.equal(await page.inputValue("#mf-from"), "2026-07-04");
    assert.equal(await page.inputValue("#mf-to"), "2026-07-05");
    await page.waitForSelector(`#md-grid [data-day="${d5}"].md-draft`);
    assert.equal(await page.$$eval("#md-grid .md-draft", (n) => n.length), 2);
    ops = await send(() => page.click('#md-form [data-mf-act="save"]'), "add an entry");
    assert.deepEqual(ops, [{ op: "add_block", page: CAL_F, type: "CalendarEntry", title: "Grillfest", params: {}, settings: { switches: grill, date: "from: 2026-07-04, to: 2026-07-05" } }]);
    assert.match(pfile(), /^vx_calendarentry_1 = CalendarEntry\("Grillfest", switches: vx_mode_1, from: 2026-07-04, to: 2026-07-05\)$/m);
    const fest = await (await page.waitForFunction(() => S.edits?.blocks?.find((b) => b.slug === "vx_calendarentry_1")?.uuid, null, { timeout: 20000 })).jsonValue();
    // Compiled: on the year in its mode's colour, its days outlined as added, listed as added.
    await page.waitForSelector(`#md-grid [data-day="${d4}"].md-add`);
    await waitText("#md-list", /Grillfest[\s\S]*added/);
    assert.equal(await page.$$eval("#md-grid .md-draft", (n) => n.length), 0);
    // Edit: another mode (Loxone Config's, by its title) and another kind of date.
    await page.click(`[data-md-edit="${fest}"]:not([disabled])`);
    assert.equal(await page.inputValue("#mf-title"), "Grillfest");
    assert.equal(await page.inputValue("#mf-mode"), grill);
    await page.selectOption("#mf-mode", PARTY);
    await page.selectOption("#mf-kind", "easter");
    await page.fill("#mf-easter", "49");
    ops = await send(() => page.click('#md-form [data-mf-act="save"]'), "edit the entry");
    assert.deepEqual(ops, [{ op: "entry_mode", uuid: fest, value: PARTY }, { op: "entry_date", uuid: fest, value: "easter: 49" }]);
    assert.match(pfile(), /^vx_calendarentry_1 = CalendarEntry\("Grillfest", switches: "Party", easter: 49\)$/m);
    await page.waitForFunction(() => S.edits?.blocks?.find((b) => b.slug === "vx_calendarentry_1")?.entry?.date === "easter: 49");
    // A date lox refuses: the server says why, and nothing changes.
    await page.click(`[data-md-edit="${fest}"]:not([disabled])`);
    await page.selectOption("#mf-kind", "yearly");
    await page.fill("#mf-yfrom", "02-30");
    await page.fill("#mf-yto", "03-01");
    await send(() => page.click('#md-form [data-mf-act="save"]'), "a refused date", 422);
    await waitText("#md-form", /no day of the year/);
    assert.match(pfile(), /easter: 49\)$/m);
    await page.click('#md-form [data-mf-act="cancel"]');
    // The mode cannot go while an entry switches it on: Properties says so.
    await undoKey(false, () => S.edits?.blocks?.find((b) => b.slug === "vx_calendarentry_1")?.entry?.date === "from: 2026-07-04, to: 2026-07-05");
    assert.match(pfile(), /switches: vx_mode_1, from: 2026-07-04, to: 2026-07-05\)$/m);
    await page.evaluate((u) => select(u, { noReveal: true }), grill);
    await page.waitForSelector('#props [data-ed="remove"][disabled]');
    assert.match(await page.getAttribute('#props [data-ed="remove"]', "title"), /Grillfest switches it on/);
    // Undone in turn: the entry, then the mode.
    await undoKey(false, () => !S.edits?.blocks.some((b) => b.slug === "vx_calendarentry_1"));
    await undoKey(false, () => !S.edits?.blocks.some((b) => b.slug === "vx_mode_1"));
    assert.doesNotMatch(pfile(), /= (Mode|CalendarEntry)\(/);
    await page.evaluate(() => modesClose());
  });
  await step("an Automatic Designer rule is shown in words, edited, compiled, undone and reverted; a new one is created and deleted (#87)", async () => {
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    await page.evaluate(() => editGoNewest());
    await settled();
    const send = async (act, what) => {
      const n = edits.length;
      await act();
      await until(() => edits.length > n && statuses[n] !== undefined, what);
      assert.equal(statuses[n], 200, what);
      await settled();
      return edits[n].ops;
    };
    const go = async (u) => { await page.evaluate((x) => { gotoPage(S.objPage.get(x), { focus: x }); select(x); }, u); };
    const sec = '#props [data-body="Rule"]';
    const file = () => readFileSync(join(dir, "pages", "vislox", "edits.loxl"), "utf8");
    const rs = `vx_${R_EIN.replace(/-/g, "_")}`, ps = `vx_${R_PUMP.replace(/-/g, "_")}`;
    // A rule Lox Logic cannot write is read-only, and says why.
    await go(R_ODD);
    await waitText(sec, /Lox Logic cannot write this rule/);
    assert.equal(await page.$(`${sec} [data-ru="open"]`), null);
    // The rule in words, its block a link.
    await go(R_EIN);
    await page.waitForSelector(`${sec} [data-ru="open"]`);
    await waitText(sec, /On weekdays and at 09:00: Brunnenpumpe: on/);
    assert.equal(await page.getAttribute(`${sec} .ru-link .lnk`, "data-go"), R_PUMP);
    // Edit: inactive, 10:00; a command without its block is not sent.
    await page.click(`${sec} [data-ru="open"]`);
    await page.uncheck(`${sec} [data-ru-f="active"]`);
    await page.fill(`${sec} [data-ru-field="at:1"]`, "10:00");
    await page.click(`${sec} [data-ru="addthen"]`);
    await waitText(sec, /Pick the block of every condition and command/);
    assert.equal(await page.getAttribute(`${sec} [data-ru="save"]`, "disabled"), "");
    await page.click(`${sec} [data-ru="del"][data-i="3"]`);
    let ops = await send(() => page.click(`${sec} [data-ru="save"]`), "the rule");
    assert.deepEqual(ops, [{ op: "rule", uuid: R_EIN, value: { active: false, lines: [
      { kind: "weekday", days: ["weekdays"] }, { kind: "time", at: "10:00" }, { kind: "then", block: R_PUMP, command: "on" }] } }]);
    assert.ok(file().includes(`rule ${rs}\n\tinactive\n\twhen weekday weekdays\n\twhen time 10:00\n\tthen ${ps} on\nend\n`), file());
    assert.match(file(), new RegExp(`^extern ${rs} = AutopilotRule\\(uuid: "${R_EIN}"\\)$`, "m"));
    await page.waitForFunction((u) => S.view.objects[u].rule?.summary === "On weekdays and at 10:00: Brunnenpumpe: on" && !S.view.objects[u].rule.active, R_EIN, { timeout: 10000 });
    await waitText(sec, /edited here[\s\S]*not deployed[\s\S]*Revert/);
    await waitText("#deploy-pend", /1 Automatic Designer rule edited here/);
    const changes = await page.evaluate(() => S.view.changes.map((c) => c.text));
    assert.ok(changes.includes("Pumpe Ein · active → inactive"), changes.join(" | "));
    assert.ok(changes.includes("Pumpe Ein · condition changed: at 09:00 → at 10:00"), changes.join(" | "));
    assert.ok(!changes.some((c) => /APC/.test(c)), changes.join(" | "));
    // Undone and redone through the reply's undo.
    ops = await undoKey(false, (u) => S.view.objects[u].rule?.active === true && !S.edits?.rules?.length, R_EIN);
    assert.deepEqual(ops, [{ op: "rule", uuid: R_EIN, value: null }]);
    assert.doesNotMatch(file(), /^rule /m);
    ops = await undoKey(true, (u) => S.view.objects[u].rule?.active === false, R_EIN);
    assert.equal(ops[0].op, "rule");
    // A draft made on a config an undo replaced is dropped, never saved over the newer rule.
    await page.waitForSelector(`${sec} [data-ru="open"]`);
    await page.click(`${sec} [data-ru="open"]`);
    await undoKey(false, (u) => S.view.objects[u].rule?.active === true, R_EIN);
    await waitText(sec, /The pending config changed while editing: the draft was dropped/);
    assert.equal(await page.$(`${sec} [data-ru="save"]`), null);
    await undoKey(true, (u) => S.view.objects[u].rule?.active === false, R_EIN);
    // Revert: back to the rule as last written, vislox's declaration goes.
    await page.waitForSelector(`${sec} [data-ru="revert"]`);
    ops = await send(() => page.click(`${sec} [data-ru="revert"]`), "revert the rule");
    assert.deepEqual(ops, [{ op: "rule", uuid: R_EIN, value: null }]);
    await page.waitForFunction((u) => S.view.objects[u].rule?.active === true, R_EIN, { timeout: 10000 });
    assert.doesNotMatch(file(), /^rule /m);
    // A new rule by its name, with a condition and an action: created below the last one, selected, then deleted.
    await page.click(`${sec} [data-ru="new"]`);
    await page.fill(`${sec} [data-ru-f="name"]`, "Pumpe Mittag");
    await waitText(sec, /Pick the block of every condition and command/);
    await page.click(`${sec} [data-ru="pick"][data-i="1"]`);
    await page.waitForSelector("#palette:not(.hidden)");
    await page.keyboard.type("Brunnen");
    await page.waitForFunction(() => /Brunnenpumpe/.test(document.querySelector("#pal-list [role=option]")?.textContent || ""));
    await page.keyboard.press("Enter");
    await page.waitForSelector("#palette.hidden", { state: "attached" });
    await page.fill(`${sec} [data-ru-field="command:1"]`, "on");
    ops = await send(() => page.click(`${sec} [data-ru="save"]`), "the new rule");
    assert.deepEqual(ops, [{ op: "new_rule", title: "Pumpe Mittag", value: { active: false, lines: [{ kind: "time", at: "08:00" }, { kind: "then", block: R_PUMP, command: "on" }] } }]);
    assert.ok(file().includes(`rule vx_rule_1 "Pumpe Mittag"\n\tinactive\n\twhen time 08:00\n\tthen ${ps} on\nend\n`), file());
    await page.waitForFunction(() => S.view.objects[S.sel]?.title === "Pumpe Mittag", null, { timeout: 10000 });
    const made = await page.evaluate(() => S.sel);
    await waitText(sec, /At 08:00: Brunnenpumpe: on/);
    await waitText(sec, /Created here as vx_rule_1/);
    // Edited, deleted, then both undone: it comes back under a new UUID, and the history follows it.
    await page.click(`${sec} [data-ru="open"]`);
    await page.check(`${sec} [data-ru-f="active"]`);
    ops = await send(() => page.click(`${sec} [data-ru="save"]`), "edit the new rule");
    assert.equal(ops[0].op, "rule");
    assert.equal(ops[0].value.active, true);
    ops = await send(() => page.click(`${sec} [data-ru="delete"]`), "delete the new rule");
    assert.deepEqual(ops, [{ op: "remove_block", uuid: made }]);
    assert.doesNotMatch(file(), /vx_rule_1/);
    await page.waitForFunction((u) => !S.view.objects[u], made, { timeout: 10000 });
    const titled = () => Object.values(S.view.objects).find((x) => x.title === "Pumpe Mittag" && x.status !== "removed");
    ops = await undoKey(false, () => Object.values(S.view.objects).find((x) => x.title === "Pumpe Mittag" && x.status !== "removed")?.rule?.active === true);
    assert.equal(ops[0].op, "new_rule");
    assert.equal(ops[0].restores, made);
    const again = await page.evaluate(`(${titled})()?.uuid`);
    assert.ok(again && again !== made, again);
    ops = await undoKey(false, (u) => S.view.objects[u]?.rule?.active === false, again);
    assert.deepEqual(ops.map((o) => [o.op, o.uuid]), [["rule", again]]);
    // And the creation itself.
    ops = await undoKey(false, (u) => !S.view.objects[u], again);
    assert.deepEqual(ops, [{ op: "remove_block", uuid: again }]);
    assert.doesNotMatch(file(), /vx_rule_1/);
  });
  await step("a new page is created in a room, marked added, takes a block, and is removed and undone (#88)", async () => {
    await page.evaluate(() => editGoNewest());
    await settled();
    const send = async (act, what) => {
      const n = edits.length;
      await act();
      await until(() => edits.length > n && statuses[n] !== undefined, what);
      assert.equal(statuses[n], 200, what);
      await settled();
      return edits[n].ops;
    };
    const file = join(dir, "pages", "vislox", "kueche-logik.loxl");
    const neu = () => page.evaluate(() => S.view?.pages.find((p) => p.title === "Küche Logik")?.uuid || null);
    // New page, next to the tabs: the base's rooms (the server's list), a title the config has not.
    await page.waitForSelector("#btn-new-page:not(.hidden):not([disabled])");
    await page.click("#btn-new-page");
    await page.waitForSelector("#newpage:not(.hidden) #np-title");
    assert.deepEqual(await page.$$eval("#np-room option", (o) => o.map((x) => x.textContent)), ["Pick a room…", "Bad", "Küche"]);
    await page.fill("#np-title", "Obergeschoss");
    await page.click('#np-body [data-np="add"]');
    await page.waitForSelector('#np-title[aria-invalid="true"]');
    assert.match(await page.textContent("#np-body"), /A page has this title already/);
    assert.match(await page.textContent("#np-body"), /lox needs a room/);
    await page.fill("#np-title", "Küche Logik");
    await page.selectOption("#np-room", "Küche");
    let ops = await send(() => page.click('#np-body [data-np="add"]'), "the new page");
    assert.deepEqual(ops, [{ op: "new_page", title: "Küche Logik", room: "Küche" }]);
    assert.equal(await page.isVisible("#newpage"), false);
    assert.ok(readFileSync(file, "utf8").endsWith('page "Küche Logik" (room: "Küche")\n'), readFileSync(file, "utf8"));
    // Shown: its tab, marked added, and the page itself.
    const made = await neu();
    assert.ok(made, "the new page is in the view");
    await page.waitForFunction((u) => S.page === u, made, { timeout: 10000 });
    await page.waitForSelector(`#tab-list .tab.added[data-uuid="${made}"][aria-selected="true"]`);
    await waitText("#deploy-pend", /1 page created here/);
    // Remove page is offered on it; Add block puts a block on it, into the same file.
    await page.waitForSelector("#btn-remove-page:not(.hidden):not([disabled])");
    await page.click("#btn-add-block");
    await page.fill("#ab-q", "monofl");
    await page.press("#ab-q", "Enter");
    await page.waitForSelector('#ab-body [data-ab-field="title"]');
    await page.fill('#ab-body [data-ab-field="title"]', "Neu");
    ops = await send(() => page.press('#ab-body [data-ab-field="title"]', "Enter"), "a block on the new page");
    assert.deepEqual(ops, [{ op: "add_block", page: made, type: "Monoflop", title: "Neu", params: {}, settings: {} }]);
    await page.waitForFunction(() => S.view?.objects[S.sel]?.title === "Neu", null, { timeout: 10000 });
    assert.equal(await page.evaluate(() => S.objPage.get(S.sel)), made);
    const text = readFileSync(file, "utf8");
    // Not deployed: found by the lock's UUID, no `# page uuid:` line yet.
    assert.doesNotMatch(text, /^# page uuid:/m);
    assert.match(text, /^page "Küche Logik" \(room: "Küche"\)\n\nvx_monoflop_\d+ = Monoflop\("Neu"\)$/m);
    // Not removed while it holds the block: the server says why.
    await page.waitForFunction(() => document.querySelector("#btn-remove-page")?.disabled && /still has what vislox added/.test(document.querySelector("#btn-remove-page").title), null, { timeout: 10000 });
    // The block undone, the page is removed: its file goes, and the tab.
    ops = await undoKey(false, () => !Object.values(S.view.objects).some((o) => o.title === "Neu" && o.status !== "removed"));
    assert.equal(ops[0].op, "remove_block");
    await page.waitForSelector("#btn-remove-page:not(.hidden):not([disabled])");
    ops = await send(() => page.click("#btn-remove-page"), "remove the page");
    assert.deepEqual(ops, [{ op: "remove_page", page: made }]);
    await page.waitForFunction((u) => !S.view.pages.some((p) => p.uuid === u), made, { timeout: 10000 });
    assert.equal(existsSync(file), false);
    await page.waitForFunction(() => !/page created here/.test(document.querySelector("#deploy-pend")?.textContent || ""));
    // Undone: created again under a new UUID; redone: the history follows it and takes that one out.
    ops = await undoKey(false, () => S.view.pages.some((p) => p.title === "Küche Logik"));
    assert.deepEqual(ops, [{ op: "new_page", title: "Küche Logik", room: "Küche", restores: made }]);
    const again = await neu();
    assert.ok(again && again !== made, again);
    ops = await undoKey(true, () => !S.view.pages.some((p) => p.title === "Küche Logik"));
    assert.deepEqual(ops, [{ op: "remove_page", page: again }]);
    assert.equal(existsSync(file), false);
  });
  await step("an older /api/edits answer never overwrites a newer one (#124)", async () => {
    await settled();
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
    const pending = () => page.evaluate(() => (S.edits?.claims || []).filter((c) => c.pending).length);
    assert.equal(await pending(), 0);
    // A load of the edits is read before the edit and answers after it: held, then released.
    let release;
    const held = new Promise((r) => { release = r; });
    let answered = false;
    await page.route("**/api/edits", async (r) => {
      if (answered) return r.continue().catch(() => {});
      answered = true;
      const resp = await r.fetch();
      await held;
      await r.fulfill({ response: resp }).catch(() => {});
    });
    try {
      await page.evaluate(() => { S.editsKey = "stale"; editsChanged(); window.__old = S.editsLoad; });
      const n = edits.length;
      await saveTitle("Neuer");
      await until(() => edits.length > n && statuses[n] !== undefined, "the save");
      assert.equal(statuses[n], 200);
      await titleIs("Neuer");
      await page.waitForFunction(() => (S.edits?.claims || []).some((c) => c.pending), null, { timeout: 15000 });
    } finally {
      release();
    }
    // The old answer is in now: it is not what S.edits says, and nothing loads it again.
    await page.evaluate(() => window.__old);
    await page.unroute("**/api/edits");
    await page.waitForTimeout(500);
    assert.ok((await pending()) > 0, "the older answer overwrote the newer one");
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 15000 });
  });
  await step("nothing was deployed: lox only compiled, checked, linted and tested", async () => {
    const ran = readFileSync(calls, "utf8").trim().split("\n");
    assert.ok(ran.length >= 2, ran.join(" | "));
    assert.ok(ran.every((l) => /^logic (compile|check --json|fmt --check|lint \.|test \.|adopt)( |$)/.test(l)), ran.join(" | "));
    // lox adopts on a copy of the module and lock, never the project's.
    assert.ok(ran.filter((l) => l.startsWith("logic adopt")).every((l) => !l.includes(join(dir, "logic.lock.json")) && !l.includes(" --module pages ")), ran.join(" | "));
  });
  await step("no errors in the browser", async () => {
    assert.deepEqual(errors, []);
  });
} catch (e) {
  failed = true;
  console.error("FAILED:", e.message);
  if (errors.length) console.error("browser errors:", errors);
  if (page) console.error("props:", await page.textContent("#props").catch(() => "?"));
  if (page) console.error("check:", await page.evaluate(() => JSON.stringify(S.check)).catch(() => "?"));
  if (page) console.error("server check:", await page.evaluate(async () => JSON.stringify([S.checkKey, S.hist?.actions?.edit, await (await fetch("/api/check")).json()])).catch(() => "?"));
  if (page) console.error("add block:", await page.textContent("#ab-body").catch(() => "?"));
  if (page) console.error("changes:", await page.textContent("#changes").catch(() => "?"));
  if (page) console.error("wire card:", await page.textContent("#wcard").catch(() => "?"), "| deploy:", await page.textContent("#deploy-msg").catch(() => "?"));
  if (page) console.error("deploy pane:", await page.textContent("#deploy-pend").catch(() => "?"));
  console.error(e.stack);
  console.error(log);
} finally {
  await browser.close();
  server.kill("SIGINT");
}
process.exit(failed ? 1 : 0);
