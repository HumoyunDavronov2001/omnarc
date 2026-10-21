# TLA+ models (SPEC NO-49)

Two models of the scheduler's protocol, checked with TLC (`tla2tools.jar` 1.8.0):

| Model | What it checks | Result |
|---|---|---|
| `Takeover.tla` + `Takeover.cfg` | A window's standby takes over while an answer is being generated (EN-17, NO-43): no answer is ever computed from a cache with a hole or a repeat, the standby's cache stays a gapless prefix, and the answer always finishes, whether the device crashes or not. | No error (N = 5: 576 states) |
| `Takeover.tla` + `TakeoverWithoutAck.cfg` | The same without the standby confirming its copies. | `NoWrongAnswer` is violated in 9 steps |
| `Replan.tla` + `Replan.cfg` | A new plan reaches every device: phones take a plan over gossip only if it is newer, never run one the computers did not agree on, never go back, and all catch up once the plan stops changing. | No error (2 phones, 3 plans: 4,736 states) |

## What the model found

Without confirmations, the device holding a window answered as soon as it had run a frame. TLC found the run where
its mirror copy is lost when it crashes a moment later: the head has moved on, resends only its last frame, and the
standby takes over with a hole in its cache. Now the device forwards its output only once the standby confirms
the copy is in its queue (`ring.rs`, `confirmed`); a standby that does not confirm within two seconds is dropped,
so a slow standby costs redundancy, never the answer.

## Running them

```bash
java -cp tla2tools.jar tlc2.TLC -config Takeover.cfg -workers 2 Takeover.tla
java -cp tla2tools.jar tlc2.TLC -config TakeoverWithoutAck.cfg -workers 2 Takeover.tla   # shows the violation
java -cp tla2tools.jar tlc2.TLC -config Replan.cfg -workers 2 Replan.tla
```
