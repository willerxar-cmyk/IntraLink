# Bounded input queue and cancellation

## Scope

Preview 3 changes only the local KVM queue and input/1 transport acceptance.
The existing authenticated wire layout, peer pins, native SendInput adapter,
video protocols, approval requirements, and maximum eight controller slots
are unchanged. No new third-party dependency, remote capability or global hook.
The wire format is unchanged, but mixed-version physical operation is not
qualified; upgrade both peers to use the new host cancellation behavior.

The controller uses one bounded 16-command mailbox per connected host.
A consecutive unsent absolute Move may replace the tail only when neither
command requests an acknowledgment and no key or mouse button is held in
the producer's ordered input state. This preserves the full drag/drawing
path, key chords, clicks, wheel events, focus barriers and acknowledgments.
Movement never crosses an intervening event. Sequence numbers are allocated
by the transport only when sending the retained command. The original wire
state still independently rejects invalid transitions and sequences.

A fresh command receives a local 500 ms budget from enqueue through the
wire acceptance ACK. Replacing an eligible pointer move uses the new
sample's time; other events retain their own age. If the queue fills with
nonreplaceable events or the budget expires, the controller fails closed
rather than silently deleting a key/button transition. Focus switching
uses the same bounded wait; the preceding host must confirm release.
Active heartbeats use a bounded acceptance budget and retain the 1 s
foreground UI lease. Inactive heartbeats retain the protocol's 3 s limit.
No timestamp is exchanged between hosts.

The host observes QUIC connection closure during the combined delivery,
sink acceptance and ACK operation. A disconnected controller invalidates
an outstanding local acceptance handle without waiting for its full
timeout. The native sink already checks that handle immediately before
application. This is cooperative cancellation, not atomic revocation of
an in-progress synchronous OS call. Normal shutdown attempts to release
owned held keys and buttons. A process crash, OS rejection or an already
accepted event still has the prior limitations.

## Verification

The transport tests cover replacement eligibility, key/ack barriers,
uncoalesced drag moves, full capacity, shutdown, expired age, canceled
receive, and actual authenticated QUIC delivery. The controlled burst test
generates 320 idle pointer samples in ten groups, retains ten idle moves,
and verifies all 83 received records, including ordered drag/key/button,
wheel and focus transitions. Its 80 send/acceptance timings use a local
test sink, not native OS injection or screen presentation. This workload
demonstrates queue behavior under accumulated unsent motion; it is not
a claim of physical bandwidth savings or human input latency.

An additional QUIC test intentionally withholds a local acceptance until
a short client deadline expires. The host must invalidate the outstanding
acceptance and the client cannot retry the uncertain record. Existing
three/eight-host and disconnect release-plan tests remain mandatory.

## Physical qualification

Only one Desktop Commander device was online during this development
round. Actual cross-machine mouse and keyboard acceptance remains pending.
Use two distinct owner-approved Windows desktops on a private IPv4 LAN,
with the host's red stop indicator visible and separate input-specific
approval. Test pointer precision, press/release, chords, wheel, drag,
rapid switching, focus loss, network interruption and ordinary local
stop. Confirm no new machine is activated before the prior release.
Observe the actual application response, not only protocol ACKs.
Do not expose the input listener publicly or enable global hooks.
