# Frontend module move verification

Base: `526966e42b06ab3704d0026b01c7737019240c14` (main).

All 840 original top-level items match byte for byte; none added or removed.
Order within each destination is unchanged. Strict mode is retained in every script.
Navigation and modes also match byte for byte apart from exported headers.

The comparison includes entire function bodies, state initializers, event handlers and startup
statements. Only file boundaries, exported headers, module contract comments and strict-mode
directives are new. HTML script order and embedded routes are checked separately by lint and the UI suites.

Reproduce from the repo root after `npm ci`:

```sh
node tools/verify-frontend-moves.mjs 526966e42b06ab3704d0026b01c7737019240c14
```

- `web/app.js`: 711 items (retained).
- `web/deploy.js`: 75 items (moved).
- `web/editing.js`: 48 items (moved).
- `web/start.js`: 6 items (moved).

| Item | Main app.js line | Destination | SHA-256 prefix |
|---|---:|---|---|
| adminToken | 2293 | web/deploy.js:20 | eace62a0e2483599 |
| act | 2302 | web/deploy.js:29 | 6ca70a0a44ed3b71 |
| deployMsg | 2316 | web/deploy.js:43 | 684ff3871476b2eb |
| editsHere | 2322 | web/deploy.js:49 | 6bbeec8374d62b10 |
| pendingChange | 2346 | web/deploy.js:73 | 8683de112b4ef56e |
| let applyAsking = false; | 2353 | web/deploy.js:80 | 5ea0ab515c8377de |
| renderActions | 2354 | web/deploy.js:81 | 838f381541c96b1a |
| renderPush | 2413 | web/deploy.js:140 | 6eff0a9fc0eb0f8b |
| renderStaleLines | 2435 | web/deploy.js:162 | 548beb5ef1d2f9dd |
| $("deploy-lines").addEventListener("click", async (e) => { | 2446 | web/deploy.js:173 | 65117fe53e7fe951 |
| $("deploy-push").addEventListener("click", async (e) => { | 2456 | web/deploy.js:183 | 67a06aae01906be4 |
| renderSteps | 2462 | web/deploy.js:189 | ea64bdba5f93f381 |
| showDeployed | 2478 | web/deploy.js:205 | 36566986cc246014 |
| hwInfo | 2511 | web/deploy.js:238 | 24cc7485b397294f |
| loadBus | 2513 | web/deploy.js:240 | 0ceef8f23455392b |
| onBus | 2528 | web/deploy.js:255 | 8dd154430f574170 |
| deployedRev | 2533 | web/deploy.js:260 | 09b8792ff5ea2fad |
| const HW_WAIT = 6000; | 2536 | web/deploy.js:263 | 9216abce85c45ff4 |
| pendingHardware | 2539 | web/deploy.js:266 | 6c7019bc60995318 |
| hwLine | 2560 | web/deploy.js:287 | f10dd3ad3924c9f1 |
| hwApplyText | 2568 | web/deploy.js:295 | 404c761f493b1a95 |
| const BUS_EVERY = 15000, BUS_FOR = 180000; | 2585 | web/deploy.js:312 | 19d14a599b8499bb |
| watchBus | 2586 | web/deploy.js:313 | 7d5ec28129bc5aee |
| deployedHardware | 2601 | web/deploy.js:328 | 08c3d417c4d86a47 |
| renderDeployed | 2618 | web/deploy.js:345 | 0a0c6b5b194e4242 |
| deployedAppCheck | 2638 | web/deploy.js:365 | 3eb6a94c5a6dcb5c |
| $("deploy-new").addEventListener("click", (e) => { | 2746 | web/deploy.js:383 | ff36b722bd10fa7e |
| $("btn-apply").onclick = async () => { | 2755 | web/deploy.js:392 | b36129e9d10b8cc9 |
| pendingScheduleText | 2762 | web/deploy.js:399 | e43d4fce92175469 |
| removing | 2774 | web/deploy.js:411 | aeb8ff28e56c6f0b |
| apply | 2775 | web/deploy.js:412 | f4ac4c01d0ec9bcc |
| confirmApply | 2791 | web/deploy.js:428 | 98b42dcb859fc8ab |
| removalsPreview | 2819 | web/deploy.js:456 | 493ec1faed2ab12a |
| dropRemovals | 2828 | web/deploy.js:465 | 2f6f6e9ed6cb8f3f |
| removalLine | 2838 | web/deploy.js:475 | 2ce7e2dd09668538 |
| removalsPlan | 2849 | web/deploy.js:486 | 2e5297f5a0bd4416 |
| askRemovals | 2859 | web/deploy.js:496 | 045a8743401dc278 |
| deploy | 2877 | web/deploy.js:514 | 9df40fa549515534 |
| let rollbackAsking = false; | 2920 | web/deploy.js:557 | a310c3dbae7fd347 |
| rollbackRow | 2922 | web/deploy.js:559 | 3360db501cf7f01f |
| rollBack | 2936 | web/deploy.js:573 | b4bf4f7829dc1094 |
| rollbackPreview | 2943 | web/deploy.js:580 | ade4af96d0468a43 |
| dropRollback | 2951 | web/deploy.js:588 | f6a90a1f51efb1fd |
| rollBackAsk | 2959 | web/deploy.js:596 | 88cf23099de81aaa |
| $("btn-sync").onclick = async () => { | 2987 | web/deploy.js:624 | d4d97bd6eafb3722 |
| $("btn-discard").onclick = async () => { | 2991 | web/deploy.js:628 | ce2bf21a721e9116 |
| rightsApplyText | 3165 | web/deploy.js:637 | fe51071cfe18c0da |
| S.check = null; | 3283 | web/deploy.js:661 | 86f20a563b2a2d20 |
| S.checkKey = null; | 3284 | web/deploy.js:662 | b9bdc25a202f1897 |
| S.checkSeq = 0; | 3285 | web/deploy.js:663 | 18d07b87cb91cc99 |
| loadCheck | 3286 | web/deploy.js:664 | 08dec06da45c78bd |
| lintAll | 3307 | web/deploy.js:685 | 1b6d2b8d41d455ee |
| lintFindings | 3309 | web/deploy.js:687 | 030b569694f596b1 |
| marksSet | 3311 | web/deploy.js:689 | e8fb73ee2e05ab38 |
| checkBroken | 3317 | web/deploy.js:695 | 61a72ad0db2f83b0 |
| checkAt | 3319 | web/deploy.js:697 | 490868926e78fafd |
| checkErrors | 3321 | web/deploy.js:699 | eaf3e56e0459588d |
| renderCheck | 3322 | web/deploy.js:700 | 58e2f5e8f5914472 |
| viewFor | 3370 | web/deploy.js:748 | f5a47b1c05574f36 |
| goButtons | 3374 | web/deploy.js:752 | 3c780114fbd80368 |
| testAll | 3386 | web/deploy.js:764 | cec1b26ff247ba0c |
| testsBad | 3388 | web/deploy.js:766 | 1e04eddefe80d4d3 |
| testBlocks | 3391 | web/deploy.js:769 | e4e2bb0117e5ecfe |
| testFindings | 3396 | web/deploy.js:774 | 2b036aa30d2ba691 |
| testsFailing | 3398 | web/deploy.js:776 | 3980534480c8591f |
| expectLine | 3405 | web/deploy.js:783 | 818db09512fe62fa |
| testLine | 3409 | web/deploy.js:787 | bb676de001852295 |
| renderTests | 3417 | web/deploy.js:795 | ae1a214ece2b7fa5 |
| $("deploy-tests").addEventListener("click", goClicked); | 3453 | web/deploy.js:831 | 7e65527bdc1f4575 |
| renderLint | 3462 | web/deploy.js:840 | 1558d39e1a32c5a1 |
| $("deploy-lint").addEventListener("click", goClicked); | 3488 | web/deploy.js:858 | 7ff6fd1ce1e168c9 |
| $("deploy-check").addEventListener("click", async (e) => { | 3489 | web/deploy.js:859 | 5521bd360a5b11f0 |
| dfltName | 4265 | web/deploy.js:873 | c1bc1614c2211a16 |
| dfltList | 4266 | web/deploy.js:874 | 949bfa728aa9dffd |
| constApplyText | 4372 | web/deploy.js:876 | 693bc0a5a5be5358 |
| S.edits = null; | 3231 | web/editing.js:21 | 184c854ab8738851 |
| S.edit = { uuid: null, open: false, draft: {}, bad: {}, busy: false, msg: "", err: "" }; | 3232 | web/editing.js:22 | 4fbc45e1517ac041 |
| S.editsKey = null; | 3233 | web/editing.js:23 | 2cd751b9bc620534 |
| S.editsSeq = 0; | 3234 | web/editing.js:24 | eb8fd8a86f3edbc4 |
| loadEdits | 3235 | web/editing.js:25 | 34916f405d907c95 |
| editsChanged | 3268 | web/editing.js:58 | 0640b521c17180a8 |
| editKey | 3503 | web/editing.js:67 | 2acd5089eac4c2e5 |
| editRev | 3506 | web/editing.js:70 | 7d65aacc3aacd96e |
| editsBlock | 3511 | web/editing.js:75 | 2c50aa972494f2c3 |
| adoptedUncommitted | 3513 | web/editing.js:77 | 31fb5ff6446cfe9b |
| editsAdded | 3515 | web/editing.js:79 | c9472a80df9f4057 |
| editsPending | 3517 | web/editing.js:81 | 4ca43a98127aad13 |
| constsPending | 3519 | web/editing.js:83 | cdec9909f72cbbee |
| editState | 3524 | web/editing.js:88 | 7f0edf277c3b3360 |
| constEditState | 4256 | web/editing.js:101 | 3876c868b9418c86 |
| S.sentFrom = null; | 6253 | web/editing.js:114 | b11f20be48e37158 |
| const SHOWING_WHY = "The last edit is still being shown. Try again in a moment."; | 6254 | web/editing.js:115 | 78385e4c7c272472 |
| showingWhy | 6255 | web/editing.js:116 | 6fa96743ae8d3ebe |
| editShowNewest | 6262 | web/editing.js:123 | 971943ba876ffba7 |
| editGoNewest | 6273 | web/editing.js:134 | 65a1d6e7fc69c837 |
| const UNDO_MAX = 50; | 6283 | web/editing.js:144 | 0a5de7341082e7e7 |
| const UNDO_LAG = 20000; | 6284 | web/editing.js:145 | 8934d02263a614e8 |
| const UNDO_QUEUE = 3; | 6285 | web/editing.js:146 | 5ba92dbab0fc2972 |
| const UNDO_KEY = "vislox.undo."; | 6286 | web/editing.js:147 | f77fc59dd1238115 |
| const UNDO = { blob: null, lastAt: null, deployed: null, done: [], undone: [], prev: null, since: 0, | 6292 | web/editing.js:153 | 2549e9948823d630 |
| const IS_MAC = /Mac\|iPhone\|iPad/.test(navigator.platform \|\| navigator.userAgent); | 6294 | web/editing.js:155 | f64064d778b6cc88 |
| const UNDO_KEYS = IS_MAC ? ["⌘Z", "⇧⌘Z"] : ["Ctrl+Z", "Shift+Ctrl+Z"]; | 6295 | web/editing.js:156 | d97506fa221f6e67 |
| undoLabel | 6297 | web/editing.js:158 | c1f1498d47495649 |
| undoNow | 6355 | web/editing.js:216 | cf6ed2acfe6ffe9f |
| undoLoad | 6359 | web/editing.js:220 | 31fca1629937fc28 |
| undoSave | 6363 | web/editing.js:224 | 55dd2a1c4e3f6909 |
| const undoSame = (was, now) => was.blob === now.blob && was.deployed === now.deployed && (now.lastAt | 6377 | web/editing.js:238 | cbd6e3d718700bfe |
| undoClear | 6379 | web/editing.js:240 | 9d3a337119b17ba0 |
| undoSync | 6385 | web/editing.js:246 | 560e0d5049cdf2d3 |
| undoRemap | 6407 | web/editing.js:268 | ce74f27b2712e1bc |
| undoSend | 6419 | web/editing.js:280 | 4ff7a25f0618f4f6 |
| editAct | 6465 | web/editing.js:326 | 52da3608b4a5f2fd |
| undoWhy | 6474 | web/editing.js:335 | 551e0b96c69f4938 |
| undoStep | 6490 | web/editing.js:351 | e7550b93fb0b2628 |
| until | 6504 | web/editing.js:365 | 7e98b2942f220a1c |
| undoRun | 6507 | web/editing.js:368 | bbcefa6188229a73 |
| renderUndo | 6541 | web/editing.js:402 | a3e9f8054ec8f718 |
| renderUndoList | 6556 | web/editing.js:417 | 6906c9cdcd5cb99c |
| undoListToggle | 6565 | web/editing.js:426 | 3a1150ea5d713440 |
| $("btn-undo").onclick = () => undoStep(false); | 6572 | web/editing.js:433 | 98784b70718ae503 |
| $("btn-redo").onclick = () => undoStep(true); | 6573 | web/editing.js:434 | c3dd9f51ce1bea8b |
| $("btn-undo-list").onclick = () => undoListToggle(); | 6574 | web/editing.js:435 | c85f14f9aed478a4 |
| document.addEventListener("click", (e) => { if (!e.target.closest("#undo-pop, #btn-undo-list")) undo | 6575 | web/editing.js:436 | 6c9c9f309b021577 |
| S.hash = readHash(); | 8483 | web/start.js:5 | 6457299534e16aff |
| loadHistory().then(connect); | 8484 | web/start.js:6 | 60b608f8c7d01dba |
| loadMs(); | 8485 | web/start.js:7 | 6521e80f2327342f |
| loadApp(); | 8486 | web/start.js:8 | d17ad8416cd455a8 |
| loadCal(); | 8487 | web/start.js:9 | f93216c9a6caae3a |
| setInterval(() => { if (!document.hidden) loadMs(false, true); }, 60 * 1000); | 8492 | web/start.js:14 | b79623424dd91cdb |
