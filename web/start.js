"use strict";
// Start only after app, editing, deployment and calendar definitions are loaded.
// These are the original app.js startup statements, in their original order.

S.hash = readHash();
loadHistory().then(connect);
loadMs();
loadApp();
loadCal();
// The chip stays roughly current while the page is open.
// Every minute, to notice a config saved on the Miniserver: the server asks only for its config
// date again (reused for half a minute, so several tabs still ask once), and the full check every
// five minutes.
setInterval(() => { if (!document.hidden) loadMs(false, true); }, 60 * 1000);
