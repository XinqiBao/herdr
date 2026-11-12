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
Full combined and platform checks remain delivery
gates; this local PTY probe does not qualify every terminal or remote transport.
When upgrading the upstream base, recheck the immutable codec boundary, optional
method advertisement, input-lease engine, encoder mode/geometry conventions and
projection/focus ordering before replaying this patch.
