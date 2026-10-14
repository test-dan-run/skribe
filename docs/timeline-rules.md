# Timeline rules

How a meeting's sections are worked out, and how ties and the edges of the recording are
treated. The code is `crates/skribe-domain/src/timeline.rs`; every rule below is locked by a
test in `crates/skribe-domain/src/timeline/tests.rs`, and the structural rules also by property
tests over generated recordings and edit sequences.

**Status.** These rules are decision D6 in
[IMPLEMENTATION_PLAN.md](../IMPLEMENTATION_PLAN.md), confirmed by the owner on 2026-10-14.
AGENTS.md refers to "established rules for tied events, zero and end boundaries, and the
initial boundary"; this document is where they are established. Changing a rule is a change to
this file, the function and its tests together.

## Inputs

The effective timeline is a pure function of four things. Nothing else is consulted and
section membership is never stored.

| Input | What it is |
|-------|------------|
| Sections | Each agenda item as it was when recording started, plus each additional section created while recording |
| Transitions | "This section starts at this time", recorded live. Append-only, each with a per-meeting sequence number |
| Overrides | Review edits, applied in their own sequence order on top of the transitions. History is never rewritten |
| End | The recording's length, once it has stopped |

Times are whole microseconds from the recording's monotonic origin. The result does not depend
on the order rows are read in: transitions and overrides are ordered by sequence number.

## Rules

1. **Derived, not stored.** The timeline is recomputed from the inputs every time.
2. **Half-open sections.** A section covers `[start, end)`: from its own boundary up to, but
   not including, the next one. Sections are contiguous and together cover the whole recording.
3. **Membership by start time.** A note belongs to the section containing its timestamp. A
   transcript segment belongs to the section containing its start time. An item stamped exactly
   on a boundary belongs to the section that starts there.
4. **Initial boundary.** The first section starts at time zero, when recording starts. It is
   the first agenda item, or "Additional section 1" when there is no agenda. It can be renamed.
   Its boundary cannot be moved or removed.
5. **Zero boundary.** No other boundary can be placed at or before zero.
6. **End boundary.** No boundary can be placed at or after the end of the recording. An item
   stamped exactly at the end, or after it, belongs to the last section.
7. **Ties.** Transitions with the same timestamp are ordered by sequence number. Two
   transitions at the same instant leave the earlier section in the structure with an empty
   range `[t, t)`; an item at that instant belongs to the later one. Nothing is dropped. A
   transition at the very end leaves an empty last section.
8. **The section shortcut.** Each trigger starts the next agenda item, in agenda order, that
   has not been started. Once every agenda item has been started, each trigger creates a new
   additional section titled "Additional section N", numbered from 1 and never reused, even
   after a rename. It never wraps to an earlier item and is never ignored.
9. **Unvisited agenda items.** An agenda item that was never started stays in the structure
   with no time range. Sections with a time range are listed first, in time order; unvisited
   agenda items follow, in agenda order.
10. **Review edits.** Renaming changes a title and nothing else: no boundary, no membership,
    no staleness. Inserting a section, setting a boundary and removing a boundary change
    boundaries, and therefore membership and minutes staleness, and are saved together in one
    transaction.

## What each edit does

| Edit | Effect | Refused when |
|------|--------|--------------|
| Rename | New title | The section does not exist |
| Set boundary (a section that has a start) | Moves its start. Time before the new start joins the section before it | It is the initial section; the time is at or before zero, or at or after the end; the time is beyond the neighbouring boundary on either side; another section starts exactly there |
| Set boundary (an unvisited agenda item) | Gives it a start, splitting the section it lands in | The time is at or before zero, or at or after the end; another section starts exactly there |
| Insert section | Adds a new section starting at the time, splitting the section it lands in | As for an unvisited agenda item |
| Remove boundary | The section's time joins the section before it. An agenda item becomes unvisited; an additional or inserted section leaves the structure | It is the initial section; the section has no start |

Two choices here go beyond the ten rules and are part of D6:

- **A moved boundary stays between its neighbours.** Moving a start past the section before or
  after it would silently reorder sections, so it is refused. To reorder, remove the boundary
  and set it again.
- **A review edit never creates a tie.** Placing a boundary exactly where another section
  starts is refused. Ties arise only from live transitions (rule 7).

An edit is checked against the timeline as it stands and saved only if accepted. Stored data
that contradicts itself (a transition for a section that does not exist, two transitions for
one section) is reported as an error, never guessed around.
