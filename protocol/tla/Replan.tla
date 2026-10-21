----------------------------- MODULE Replan -----------------------------
(***************************************************************************)
(* How a new plan reaches every device (SPEC NO-43, NO-49).                *)
(*                                                                         *)
(* The computers agree on each plan through Raft; its version only grows.  *)
(* The leader repeats the plan over gossip now and then, and a device that *)
(* keeps no log (a phone) takes a plan it hears only if its version is     *)
(* newer than the one it runs. Gossip loses messages and delivers them in  *)
(* any order; devices go offline and come back.                            *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    Phones,     \* devices that keep no log
    MaxVersion  \* plans the computers agree on

VARIABLES
    agreed,     \* the newest plan in the computers' log
    running,    \* [phone |-> the plan version it runs]
    inflight,   \* gossip messages on their way: [to, version]
    up          \* phones online

vars == <<agreed, running, inflight, up>>

Init ==
    /\ agreed = 1
    /\ running = [p \in Phones |-> 0]
    /\ inflight = {}
    /\ up = Phones

\* the computers agree on a new plan: a device left, joined, or the owner started another model
Agree ==
    /\ agreed < MaxVersion
    /\ agreed' = agreed + 1
    /\ UNCHANGED <<running, inflight, up>>

\* the leader repeats the plan to every phone
Spread ==
    /\ inflight' = inflight \cup {[to |-> p, version |-> agreed] : p \in Phones}
    /\ UNCHANGED <<agreed, running, up>>

\* a message arrives: only a newer plan replaces the one running
Hear(m) ==
    /\ m \in inflight
    /\ m.to \in up
    /\ running' = [running EXCEPT ![m.to] = IF m.version > @ THEN m.version ELSE @]
    /\ inflight' = inflight \ {m}
    /\ UNCHANGED <<agreed, up>>

Lose(m) ==
    /\ m \in inflight
    /\ inflight' = inflight \ {m}
    /\ UNCHANGED <<agreed, running, up>>

GoOffline(p) == /\ p \in up /\ up' = up \ {p} /\ UNCHANGED <<agreed, running, inflight>>
ComeBack(p) == /\ p \notin up /\ up' = up \cup {p} /\ UNCHANGED <<agreed, running, inflight>>

Next ==
    \/ Agree \/ Spread
    \/ \E m \in inflight : Hear(m) \/ Lose(m)
    \/ \E p \in Phones : GoOffline(p) \/ ComeBack(p)

\* the leader keeps repeating; phones come back; the network does not lose every message to a phone
Fairness ==
    /\ WF_vars(Spread)
    /\ \A p \in Phones : WF_vars(ComeBack(p))
    /\ \A p \in Phones : SF_vars(\E m \in inflight : m.to = p /\ Hear(m))

Spec == Init /\ [][Next]_vars /\ Fairness

\* the message set stays finite for TLC
Bounded == Cardinality(inflight) <= Cardinality(Phones) * MaxVersion

\* Safety: no phone runs a plan the computers have not agreed on
OnlyAgreed == \A p \in Phones : running[p] <= agreed

\* Safety: a phone never goes back to an older plan
NeverBack == [][\A p \in Phones : running'[p] >= running[p]]_vars

\* Liveness: once the computers stop changing the plan and the phones stay online, every phone runs the newest one
\* (a phone that keeps dropping off the network as each repetition arrives can miss them all)
EveryoneCatchesUp == <>[](up = Phones) => <>[](\A p \in Phones : running[p] = agreed)

=============================================================================
