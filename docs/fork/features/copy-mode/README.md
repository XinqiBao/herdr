# Mouse history browsing and copy navigation

This fork extends the stable copy-mode client with viewport navigation and mouse
history capture. Copy presentation belongs to the client; terminal input routing
remains authoritative on the server. Stable pane IDs identify every operation.

The independent feature branch starts at upstream v0.9.5 and extends the prior
viewport-navigation delivery. It has no Goto, tab-navigation or fork CI dependency.

## Behavior

On a server advertising `pane.wheel`, scrolling upward in Herdr history enters
copy mode. A downward host scroll enters when already viewing history. Mouse
input and the next keyboard events are reserved before asynchronous focus or
wheel requests; immediate Ctrl+D becomes copy-mode half-page navigation rather
than a foreground-process control byte. Scrolling an empty history upward still
captures input. Reaching the bottom, continued output, resizing and temporary
surface gaps do not exit copy mode.

Scrollbar browsing is explicit host viewport navigation and enters copy mode
before requesting its absolute offset. The initial offset is retained, including
zero when entry began at live output. Existing deliberate exits restore it:
q exits, y/Enter copy and exit, and Esc clears active selection/search before a
later Esc exits. Prefix navigation keeps its existing meaning and can park the
single copy session across focus changes.

H/M/L move to the top/middle/bottom of the visible viewport without scrolling.
Ctrl+e/y scroll one line while keeping the cursor on its text until an edge moves
it. Existing motions, search, selection and copy behavior remain available.

Application MouseReport and AlternateScroll wheel routes preserve application
input. While the route is unresolved, ordinary q/Esc are queued input; they are
interpreted as copy exits only after host routing is confirmed. An explicit
prefix/navigation action cancels the pending input context and invalidates its
reply. It never forwards keys held under an uncertain history operation.

If A retains a copy session while the wheel targets application pane B, A is
parked without changing its cursor, selection, search or entry offset. B's
confirmed application context applies to queued and subsequent keys, text and
paste even before the focus snapshot arrives. An intervening snapshot still
showing A cannot resume A's copy routing. Focus confirmation or deliberate
navigation retires this local context; boot, endpoint and target invalidation
also retire it. A downward host wheel at live output follows the same non-entry
input path. Pending copy operations from A are retired when parking, so their
late replies cannot modify the retained session or release old queued keys.

## Protocol and ordering

`pane.wheel` is a separately advertised public JSON method with explicit pane,
vertical direction, line count, modifiers, cell fallback and optional pixel
geometry. The operation chooses HostScroll, MouseReport or AlternateScroll under
one terminal-core lock, and applies that decision without a second mode query.
Its result identifies the pane and actual route, pre/post scroll metrics, terminal
geometry, active screen and content revision. It does not change focus itself.

Pixel coordinates are accepted only while live SGR pixel mode is enabled and
the geometry/bounds match; otherwise the supplied cell coordinates apply. For
application routes the encoded bytes must enter the nonblocking input channel
before resetting the viewport. Rejected admission preserves its original offset.
Encoding uses the existing terminal mouse/key encoders and lock order.

The client keeps one ordered pending stream, bounded to 256 events and 64 KiB
of retained text/key payload. Unresolved wheel events are never coalesced: each
application wheel represents one request. Adjacent host-copy scroll offsets use
the existing coalescing pipeline only after capture is established. Old forwarded
leases close once at their recorded targets; replay uses the normal key/release
engine with the reserved pane, rather than resolving focus again.
If a later wheel fails or is cancelled after an earlier application replay has
forwarded a key press or mouse-down, the client closes that established ownership
once. Pending repeats, new presses, text and paste are discarded. Closures use
the original endpoint, boot and pane and pass through the normal surface fence,
including callback, rejected admission and command-lane retirement paths. A
disconnected/fenced target cannot guarantee remote release delivery; closures
are never retargeted or retained for replay into a replacement connection.
Keys preceding the first wheel and releases closing old leases are dispatched
before the wheel request as well. Selection uses the reserved copy viewport;
scrollbar hit testing retains the displayed thumb geometry until a new projection
arrives, and a final drag release updates the local copy offset even while an
earlier absolute scroll is still in flight.

Unknown results, timeout and queue overflow discard uncertain queued input and
retain a visible local paused state until deliberate Esc/q or prefix navigation.
They do not retry an operation that might already have applied. One application
per received request is guaranteed; delivery across a lost response is uncertain.
Endpoint/boot changes, pane removal and invalidated generations reject old replies.

## Compatibility and validation

Frozen generation-1 snapshot, surface, semantic-input and blob codecs remain
unchanged, as do existing method shapes and their fixtures. The new method is
frozen separately. An older endpoint without it retains legacy application wheel
input and explicit prefix+[ copy mode. Automatic wheel capture and immediate-key
protection are unavailable there; the client shows a bounded local notice rather
than guessing a route from incomplete surface flags or rejecting the connection.

Focused deterministic tests cover authoritative routing precedence, alternate
scroll modes and bytes, offsets, pixel fallback/mode transitions, input admission
failure, explicit targets, same/separate-batch immediate Ctrl+D, application event
order, host selection versus application clicks, unknown outcomes, bounded queues,
late replies, projection gaps and existing viewport navigation. Isolated runtime
verification with a raw recording child observed no Ctrl+D from host history
capture in one or separate input batches, including after returning to bottom
under continuing output. MouseReport preserved wheel-up, q, wheel-down, text in
order, once each. AlternateScroll preserved application-cursor Up, Ctrl+D, Down,
q in order. The disposable session was stopped and deleted afterward.

Formatting, clippy and 527 related client/API/runtime/protocol tests passed for
the initial implementation; the final 18 capture regressions also passed,
including queued pixel preservation and explicit navigation during focus wait.
Follow-up regressions exercise native/VT ownership across second-wheel failures,
event/byte overflow, explicit cancellation and focus loss; recording transports
verify actual guarded key/mouse closure delivery and suppression across invalid
identities, frozen surfaces and failed/replaced connections. Active A copy state
is characterized across both application routes and host-down without entry,
both response/focus orderings and continued input before focus confirmation.
The Linux follow-up passed all 412 related client shell/runtime tests (including
24 capture tests), formatting, all-target clippy with warnings denied, and six
architecture contracts. This is focused evidence; the complete native suite,
including Linux live handoff, remains a coordinated integration check.
Full combined and platform checks remain delivery
gates; this local PTY probe does not qualify every terminal or remote transport.
When upgrading the upstream base, recheck the immutable codec boundary, optional
method advertisement, input-lease engine, encoder mode/geometry conventions and
projection/focus ordering before replaying this patch.
