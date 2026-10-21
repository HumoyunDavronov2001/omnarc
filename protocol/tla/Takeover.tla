---------------------------- MODULE Takeover ----------------------------
(***************************************************************************)
(* A window's standby taking over while an answer is being generated       *)
(* (SPEC EN-17, NO-43, NO-49).                                             *)
(*                                                                         *)
(* The head sends one token position at a time to the device P that holds  *)
(* a window, and the next position only once the answer for the last one  *)
(* came back. P runs each frame and sends a copy (MIRROR) to the standby   *)
(* R, which runs it to keep its KV cache in step and forwards nothing. P   *)
(* may crash at any moment; bytes it had not sent yet are lost with it.    *)
(* The head then resends its last frame to R marked REPLAY: R cuts its     *)
(* cache at that position, runs the frame and is the live device from now *)
(* on; mirror copies still on their way are dropped once R is live.       *)
(*                                                                         *)
(* With Ack = FALSE, P answers as soon as it has run a frame; with         *)
(* Ack = TRUE, it answers only once R has taken the frame's copy off the   *)
(* wire. The model shows why the second is needed: without it a copy can   *)
(* still be in flight, or be lost with P, while the head moves on, and R   *)
(* takes over with a hole in its cache.                                    *)
(*                                                                         *)
(* A prompt goes out in chunks without waiting (Pipeline > 1); then the     *)
(* head must resend every step not yet answered (ReplayAll), or a chunk P  *)
(* took with it never reaches R.                                           *)
(***************************************************************************)
EXTENDS Naturals, Sequences

CONSTANTS
    N,          \* positions the head generates
    Ack,        \* P answers only once R holds the copy
    Pipeline,   \* steps the head sends before an answer comes back: 1 while decoding, more for a prompt's chunks
    ReplayAll   \* the head resends every unanswered step to R, not just the last one

Frame(p, mirrored, replay) == [pos |-> p, mirror |-> mirrored, replay |-> replay]
Prefix(k) == [i \in 1..k |-> i]
Range(s) == {s[i] : i \in DOMAIN s}
Min(S) == CHOOSE x \in S : \A y \in S : x <= y

VARIABLES
    sent,       \* positions the head has sent: 1..sent
    answered,   \* positions whose answer came back to the head
    kvP, kvR,   \* the caches: the positions each device ran, in order
    inP, inR,   \* frames read off the wire, queued for the device's engine
    wireP,      \* head -> P, in flight
    wireM,      \* P -> R, mirror copies in flight
    wireR,      \* head -> R, in flight, after the takeover
    upP,        \* P is up
    target,     \* where the head sends
    liveR,      \* R took over
    waiting,    \* with Ack: positions P ran and answers once R holds their copy
    wrong       \* an answer was computed from a cache that is not exactly 1..p

vars == <<sent, answered, kvP, kvR, inP, inR, wireP, wireM, wireR, upP, target, liveR, waiting, wrong>>

Init ==
    /\ sent = 0 /\ answered = 0
    /\ kvP = <<>> /\ kvR = <<>>
    /\ inP = <<>> /\ inR = <<>>
    /\ wireP = <<>> /\ wireM = <<>> /\ wireR = <<>>
    /\ upP = TRUE /\ target = "P" /\ liveR = FALSE
    /\ waiting = {} /\ wrong = FALSE

\* the head sends the next position while fewer than Pipeline steps wait for an answer
HeadSend ==
    /\ sent < N /\ sent - answered < Pipeline
    /\ sent' = sent + 1
    /\ IF target = "P"
         THEN /\ wireP' = Append(wireP, Frame(sent + 1, FALSE, FALSE)) /\ UNCHANGED wireR
         ELSE /\ wireR' = Append(wireR, Frame(sent + 1, FALSE, FALSE)) /\ UNCHANGED wireP
    /\ UNCHANGED <<answered, kvP, kvR, inP, inR, wireM, upP, target, liveR, waiting, wrong>>

ReadP ==
    /\ upP /\ wireP /= <<>>
    /\ inP' = Append(inP, Head(wireP)) /\ wireP' = Tail(wireP)
    /\ UNCHANGED <<sent, answered, kvP, kvR, inR, wireM, wireR, upP, target, liveR, waiting, wrong>>

\* R reads from two connections: which frame enters its queue first is up to the network
ReadMirror ==
    /\ wireM /= <<>>
    /\ inR' = Append(inR, Head(wireM)) /\ wireM' = Tail(wireM)
    /\ UNCHANGED <<sent, answered, kvP, kvR, inP, wireP, wireR, upP, target, liveR, waiting, wrong>>

ReadHead ==
    /\ wireR /= <<>>
    /\ inR' = Append(inR, Head(wireR)) /\ wireR' = Tail(wireR)
    /\ UNCHANGED <<sent, answered, kvP, kvR, inP, wireP, wireM, upP, target, liveR, waiting, wrong>>

\* P runs a frame, mirrors it, and answers (at once, or once R holds the copy)
RunP ==
    /\ upP /\ inP /= <<>>
    /\ LET f == Head(inP) IN
        /\ kvP' = Append(kvP, f.pos)
        /\ wireM' = Append(wireM, Frame(f.pos, TRUE, FALSE))
        /\ wrong' = (wrong \/ Append(kvP, f.pos) /= Prefix(f.pos))
        /\ IF Ack
             THEN /\ waiting' = waiting \cup {f.pos} /\ UNCHANGED answered
             ELSE /\ answered' = f.pos /\ UNCHANGED waiting
    /\ inP' = Tail(inP)
    /\ UNCHANGED <<sent, kvR, inR, wireP, wireR, upP, target, liveR>>

\* with Ack: R took the copy off the wire, so P answers; answers go back in order, on one stream
AckAnswer ==
    /\ upP
    /\ waiting /= {}
    /\ LET p == Min(waiting) IN
        /\ p \in Range(kvR) \cup {inR[i].pos : i \in DOMAIN inR}
        /\ answered' = p
        /\ waiting' = waiting \ {p}
    /\ UNCHANGED <<sent, kvP, kvR, inP, inR, wireP, wireM, wireR, upP, target, liveR, wrong>>

RunR ==
    /\ inR /= <<>>
    /\ LET f == Head(inR) IN
        IF f.mirror /\ liveR
          \* a copy still on its way when R became live: the step is in its cache already
          THEN UNCHANGED <<kvR, liveR, answered, wrong>>
        ELSE IF f.replay
          \* the resent step: cut the cache there, run it, and answer from now on
          THEN LET kv == Append(SubSeq(kvR, 1, IF f.pos - 1 < Len(kvR) THEN f.pos - 1 ELSE Len(kvR)), f.pos) IN
                /\ kvR' = kv /\ liveR' = TRUE /\ answered' = f.pos
                /\ wrong' = (wrong \/ kv /= Prefix(f.pos))
        ELSE IF f.mirror
          THEN /\ kvR' = Append(kvR, f.pos) /\ UNCHANGED <<liveR, answered, wrong>>
        ELSE LET kv == Append(kvR, f.pos) IN
                /\ kvR' = kv /\ liveR' = TRUE /\ answered' = f.pos
                /\ wrong' = (wrong \/ kv /= Prefix(f.pos))
    /\ inR' = Tail(inR)
    /\ UNCHANGED <<sent, kvP, inP, wireP, wireM, wireR, upP, target, waiting>>

\* P crashes once: what it queued is gone, and so are the copies it had not sent yet
Crash ==
    /\ upP
    /\ upP' = FALSE /\ inP' = <<>> /\ wireP' = <<>> /\ waiting' = {}
    /\ \E k \in 0..Len(wireM) : wireM' = SubSeq(wireM, 1, k)
    /\ UNCHANGED <<sent, answered, kvP, kvR, inR, wireR, target, liveR, wrong>>

\* the head notices and resends its last step to R, which stands in for P
Takeover ==
    /\ ~upP /\ target = "P"
    /\ target' = "R"
    /\ IF sent > answered
         THEN wireR' = wireR \o
                IF ReplayAll
                  THEN [i \in 1..(sent - answered) |-> Frame(answered + i, FALSE, TRUE)]
                  ELSE <<Frame(sent, FALSE, TRUE)>>
         ELSE UNCHANGED wireR
    /\ UNCHANGED <<sent, answered, kvP, kvR, inP, inR, wireP, wireM, upP, liveR, waiting, wrong>>

\* the answer is complete: nothing more happens
Done == answered = N /\ UNCHANGED vars

Next == HeadSend \/ ReadP \/ ReadMirror \/ ReadHead \/ RunP \/ AckAnswer \/ RunR \/ Crash \/ Takeover \/ Done

\* every step but the crash happens once it can; the crash may or may not
Spec == Init /\ [][Next]_vars
        /\ WF_vars(HeadSend) /\ WF_vars(ReadP) /\ WF_vars(ReadMirror) /\ WF_vars(ReadHead)
        /\ WF_vars(RunP) /\ WF_vars(AckAnswer) /\ WF_vars(RunR) /\ WF_vars(Takeover)

\* Safety: every answer comes from a cache holding exactly the positions before it, once each
NoWrongAnswer == ~wrong

\* Safety: the standby's cache never has a hole or a repeat
StandbyInStep == kvR = Prefix(Len(kvR))

\* Liveness: the answer is finished, whether P crashed or not
Finishes == <>(answered = N)

=============================================================================
