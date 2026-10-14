//! The effective timeline: which sections a meeting has and when each one ran.
//!
//! The rules are recorded in `docs/timeline-rules.md`. The timeline is a pure function of the
//! sections planned or created, the transitions recorded live and the edits made in review.
//! Section membership is never stored; it is derived here every time.

use std::collections::HashSet;
use std::fmt;

use crate::SectionId;

/// Microseconds from the recording's monotonic origin.
pub type Micros = i64;

/// Where a section came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionOrigin {
    /// Planned before recording. `position` is its place in the agenda, from zero.
    Agenda { position: u32 },
    /// Created during recording once the agenda was exhausted, or because there was none.
    Additional { number: u32 },
    /// Created in review by inserting a boundary.
    Inserted,
}

/// A section as it was planned or created, before any review edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionSeed {
    pub id: SectionId,
    pub origin: SectionOrigin,
    pub title: String,
}

/// A recorded section transition. Transitions are history: they are never changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    /// Persisted per-meeting order. It decides between transitions at the same instant.
    pub sequence: u64,
    pub at: Micros,
    /// The section that starts at `at`.
    pub section: SectionId,
}

/// A review edit, applied on top of the recorded transitions in `sequence` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Override {
    pub sequence: u64,
    pub edit: Edit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Changes the title only. Boundaries, membership and staleness are untouched.
    Rename { section: SectionId, title: String },
    /// Moves a section's start, or gives an unvisited section a start.
    SetBoundary { section: SectionId, at: Micros },
    /// Removes a section's start. Its time joins the section before it.
    RemoveBoundary { section: SectionId },
    /// Adds a new section starting at `at`.
    InsertSection {
        section: SectionId,
        at: Micros,
        title: String,
    },
}

/// When a section ran: the half-open interval `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    pub start: Micros,
    /// `None` for the last section while the recording is still running.
    pub end: Option<Micros>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSection {
    pub id: SectionId,
    pub origin: SectionOrigin,
    pub title: String,
    /// `None` for an agenda item that was never reached.
    pub range: Option<TimeRange>,
}

/// The stored sections, transitions and edits contradict each other. Edits are checked before
/// they are saved, so this means damaged data rather than a user mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineError {
    DuplicateSection(SectionId),
    UnknownSection(SectionId),
    /// Two transitions start the same section.
    DuplicateTransition(SectionId),
    /// An edit removes the start of a section that has none.
    NoBoundary(SectionId),
}

impl fmt::Display for TimelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSection(id) => write!(f, "section {id} is defined twice"),
            Self::UnknownSection(id) => write!(f, "section {id} is not defined"),
            Self::DuplicateTransition(id) => write!(f, "section {id} starts twice"),
            Self::NoBoundary(id) => write!(f, "section {id} has no start to remove"),
        }
    }
}

impl std::error::Error for TimelineError {}

/// Why a review edit is not allowed. Nothing has changed when one of these is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    UnknownSection,
    /// The first section starts when the recording starts. It can be renamed only.
    InitialBoundary,
    /// No other boundary can sit at or before the start of the recording.
    NotAfterStart,
    /// No boundary can sit at or after the end of the recording.
    NotBeforeEnd,
    /// Another section already starts at that instant.
    BoundaryAlreadyThere,
    /// A section's start can move only between the sections on either side of it.
    PastNeighbour,
    /// The section was never reached, so it has no start to remove.
    NoBoundary,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownSection => "the section does not exist",
            Self::InitialBoundary => "the first section starts with the recording",
            Self::NotAfterStart => "a boundary must be after the start of the recording",
            Self::NotBeforeEnd => "a boundary must be before the end of the recording",
            Self::BoundaryAlreadyThere => "another section already starts at that time",
            Self::PastNeighbour => "a boundary can move only between its neighbours",
            Self::NoBoundary => "the section has no start to remove",
        })
    }
}

impl std::error::Error for EditError {}

/// Sections in display order: those with a time range chronologically, then agenda items that
/// were never reached, in agenda order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveTimeline {
    sections: Vec<EffectiveSection>,
    timed: usize,
    end: Option<Micros>,
}

/// Where a boundary sorts among boundaries at the same instant: recorded transitions first, in
/// their persisted order, then boundaries placed in review, in edit order.
type Tie = (u8, u64);

struct Draft {
    seed: SectionSeed,
    boundary: Option<(Micros, Tie)>,
}

/// Derives the effective timeline. `end` is the recording's length once it has stopped.
///
/// The result does not depend on the order of the slices: transitions and overrides are
/// ordered by their sequence numbers.
pub fn derive(
    seeds: &[SectionSeed],
    transitions: &[Transition],
    overrides: &[Override],
    end: Option<Micros>,
) -> Result<EffectiveTimeline, TimelineError> {
    let mut drafts: Vec<Draft> = Vec::with_capacity(seeds.len());
    let mut known = HashSet::with_capacity(seeds.len());
    for seed in seeds {
        if !known.insert(seed.id) {
            return Err(TimelineError::DuplicateSection(seed.id));
        }
        drafts.push(Draft {
            seed: seed.clone(),
            boundary: None,
        });
    }

    for transition in transitions {
        let draft = find(&mut drafts, transition.section)?;
        if draft.boundary.is_some() {
            return Err(TimelineError::DuplicateTransition(transition.section));
        }
        draft.boundary = Some((transition.at, (0, transition.sequence)));
    }

    let mut ordered: Vec<&Override> = overrides.iter().collect();
    ordered.sort_by_key(|entry| entry.sequence);
    for entry in ordered {
        match &entry.edit {
            Edit::Rename { section, title } => {
                find(&mut drafts, *section)?.seed.title.clone_from(title);
            }
            Edit::SetBoundary { section, at } => {
                let draft = find(&mut drafts, *section)?;
                // A moved boundary keeps its place among ties; a new one sorts as an edit.
                let tie = draft.boundary.map_or((1, entry.sequence), |(_, tie)| tie);
                draft.boundary = Some((*at, tie));
            }
            Edit::RemoveBoundary { section } => {
                let draft = find(&mut drafts, *section)?;
                if draft.boundary.take().is_none() {
                    return Err(TimelineError::NoBoundary(*section));
                }
            }
            Edit::InsertSection { section, at, title } => {
                if !known.insert(*section) {
                    return Err(TimelineError::DuplicateSection(*section));
                }
                drafts.push(Draft {
                    seed: SectionSeed {
                        id: *section,
                        origin: SectionOrigin::Inserted,
                        title: title.clone(),
                    },
                    boundary: Some((*at, (1, entry.sequence))),
                });
            }
        }
    }

    let (mut timed, untimed): (Vec<Draft>, Vec<Draft>) = drafts
        .into_iter()
        .partition(|draft| draft.boundary.is_some());
    timed.sort_by_key(|draft| draft.boundary);

    let starts: Vec<Micros> = timed
        .iter()
        .filter_map(|draft| draft.boundary.map(|(at, _)| at))
        .collect();
    let mut sections: Vec<EffectiveSection> = Vec::with_capacity(timed.len() + untimed.len());
    for (index, draft) in timed.into_iter().enumerate() {
        let start = starts[index];
        let range_end = match starts.get(index + 1) {
            Some(next) => Some(*next),
            // A boundary recorded at the very end leaves an empty last section, not a
            // negative one.
            None => end.map(|end| end.max(start)),
        };
        sections.push(EffectiveSection {
            id: draft.seed.id,
            origin: draft.seed.origin,
            title: draft.seed.title,
            range: Some(TimeRange {
                start,
                end: range_end,
            }),
        });
    }
    let timed_count = sections.len();

    // Agenda items never reached stay in the structure. A section created during recording or
    // review whose boundary was removed has neither a plan nor a time, so it is left out.
    let mut unvisited: Vec<(u32, Draft)> = untimed
        .into_iter()
        .filter_map(|draft| match draft.seed.origin {
            SectionOrigin::Agenda { position } => Some((position, draft)),
            SectionOrigin::Additional { .. } | SectionOrigin::Inserted => None,
        })
        .collect();
    unvisited.sort_by_key(|(position, _)| *position);
    sections.extend(unvisited.into_iter().map(|(_, draft)| EffectiveSection {
        id: draft.seed.id,
        origin: draft.seed.origin,
        title: draft.seed.title,
        range: None,
    }));

    Ok(EffectiveTimeline {
        sections,
        timed: timed_count,
        end,
    })
}

fn find(drafts: &mut [Draft], id: SectionId) -> Result<&mut Draft, TimelineError> {
    drafts
        .iter_mut()
        .find(|draft| draft.seed.id == id)
        .ok_or(TimelineError::UnknownSection(id))
}

impl EffectiveTimeline {
    /// Every section in display order.
    pub fn sections(&self) -> &[EffectiveSection] {
        &self.sections
    }

    /// The sections that have a time range, in chronological order.
    pub fn timed(&self) -> &[EffectiveSection] {
        &self.sections[..self.timed]
    }

    /// Agenda items that were never reached, in agenda order.
    pub fn unvisited(&self) -> &[EffectiveSection] {
        &self.sections[self.timed..]
    }

    /// The recording's length, once it has stopped.
    pub fn end(&self) -> Option<Micros> {
        self.end
    }

    /// The section a note or transcript segment stamped `at` belongs to: the last one that
    /// starts at or before it. So an item on a boundary belongs to the section starting
    /// there, an item at the very end belongs to the last section, and of two sections
    /// starting at the same instant the later one receives the item.
    ///
    /// `None` only when no section has started.
    pub fn section_at(&self, at: Micros) -> Option<&EffectiveSection> {
        let timed = self.timed();
        let after = timed.partition_point(|section| start_of(section) <= at);
        // An item stamped before the first boundary still belongs to the first section.
        timed.get(after.saturating_sub(1))
    }

    /// Whether `section`'s start may be set to `at`: moved, or placed for an agenda item that
    /// was never reached.
    pub fn check_set_boundary(&self, section: SectionId, at: Micros) -> Result<(), EditError> {
        let index = self.position(section)?;
        if index >= self.timed {
            return self.check_new_boundary(at);
        }
        if index == 0 {
            return Err(EditError::InitialBoundary);
        }
        self.check_inside_recording(at)?;
        let timed = self.timed();
        let previous = start_of(&timed[index - 1]);
        let next = timed.get(index + 1).map(start_of);
        if at == previous || next == Some(at) {
            return Err(EditError::BoundaryAlreadyThere);
        }
        if at < previous || next.is_some_and(|next| at > next) {
            return Err(EditError::PastNeighbour);
        }
        Ok(())
    }

    /// Whether a new section may be inserted starting at `at`.
    pub fn check_insert_section(&self, at: Micros) -> Result<(), EditError> {
        self.check_new_boundary(at)
    }

    /// Whether `section`'s start may be removed.
    pub fn check_remove_boundary(&self, section: SectionId) -> Result<(), EditError> {
        let index = self.position(section)?;
        if index >= self.timed {
            return Err(EditError::NoBoundary);
        }
        if index == 0 {
            return Err(EditError::InitialBoundary);
        }
        Ok(())
    }

    fn position(&self, section: SectionId) -> Result<usize, EditError> {
        self.sections
            .iter()
            .position(|candidate| candidate.id == section)
            .ok_or(EditError::UnknownSection)
    }

    fn check_inside_recording(&self, at: Micros) -> Result<(), EditError> {
        let start = self.timed().first().map_or(0, start_of);
        if at <= start {
            return Err(EditError::NotAfterStart);
        }
        if self.end.is_some_and(|end| at >= end) {
            return Err(EditError::NotBeforeEnd);
        }
        Ok(())
    }

    fn check_new_boundary(&self, at: Micros) -> Result<(), EditError> {
        self.check_inside_recording(at)?;
        if self.timed().iter().any(|section| start_of(section) == at) {
            return Err(EditError::BoundaryAlreadyThere);
        }
        Ok(())
    }
}

fn start_of(section: &EffectiveSection) -> Micros {
    section.range.map_or(0, |range| range.start)
}

/// What the section shortcut does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// Start this agenda item: the next one, in agenda order, that has not been started.
    Agenda(SectionId),
    /// The agenda is exhausted or empty: create a new additional section with this number.
    Additional { number: u32 },
}

/// Decides the next transition while recording, including the first one when recording
/// starts. It never wraps to an earlier item and never declines: once every agenda item has
/// been started, each call asks for one more additional section.
pub fn next_advance(seeds: &[SectionSeed], transitions: &[Transition]) -> Advance {
    let started: HashSet<SectionId> = transitions
        .iter()
        .map(|transition| transition.section)
        .collect();
    let next_agenda = seeds
        .iter()
        .filter(|seed| !started.contains(&seed.id))
        .filter_map(|seed| match seed.origin {
            SectionOrigin::Agenda { position } => Some((position, seed.id)),
            SectionOrigin::Additional { .. } | SectionOrigin::Inserted => None,
        })
        .min_by_key(|(position, _)| *position);
    if let Some((_, id)) = next_agenda {
        return Advance::Agenda(id);
    }
    let highest = seeds
        .iter()
        .filter_map(|seed| match seed.origin {
            SectionOrigin::Additional { number } => Some(number),
            SectionOrigin::Agenda { .. } | SectionOrigin::Inserted => None,
        })
        .max()
        .unwrap_or(0);
    Advance::Additional {
        number: highest.saturating_add(1),
    }
}

/// The default title of an additional section. The user can rename it.
pub fn additional_section_title(number: u32) -> String {
    format!("Additional section {number}")
}

#[cfg(test)]
mod tests;
