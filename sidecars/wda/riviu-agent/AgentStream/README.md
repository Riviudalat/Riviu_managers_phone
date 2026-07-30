# MJPEG Contract

Project 2 hardens the WDA 15.1.4 MJPEG implementation. Candidate device port
`9094` is supplied at DVT launch through `MJPEG_SERVER_PORT`, binds only to the
device loopback interface, and requires `X-Riviu-Token` before registering a
stream client.

The required lifecycle is strict:

1. Cold-launch candidate with control and MJPEG port environment.
2. Open only the control relay.
3. Verify protected health and foreground the target app.
4. Create a fresh automation session.
5. Open the MJPEG relay and wait for the first complete JPEG.

`probe_gate_bc.py` keeps one authenticated MJPEG sampler for gesture evidence and
stability measurement. Every frame is decoded as JPEG with Pillow. Gesture proof
uses mean absolute luma change in a defined Settings region and a preceding
no-action frame as its control; it does not poll WDA screenshots. The five-minute
gate also measures frame cadence, maximum gap, bounded reconnects, protected
health, and the active session at fixed intervals. Connection open, HTTP headers,
marker-only payloads, or one early frame do not satisfy the gate.

Current live result: `PENDING_MAC_DEVICE`.
