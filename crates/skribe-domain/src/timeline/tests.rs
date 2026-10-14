use proptest::prelude::*;

use super::*;

fn agenda(titles: &[&str]) -> Vec<SectionSeed> {
    titles
        .iter()
        .zip(0u32..)
        .map(|(title, position)| SectionSeed {
            id: SectionId::generate(),
            origin: SectionOrigin::Agenda { position },
            title: (*title).to_owned(),
        })
        .collect()
}

/// A recording in progress, driven only through [`next_advance`] the way the app drives it.
#[derive(Debug, Clone)]
struct Live {
    seeds: Vec<SectionSeed>,
    transitions: Vec<Transition>,
}

impl Live {
    /// Starts recording: the first section starts at time zero.
    fn start(seeds: Vec<SectionSeed>) -> Self {
        let mut live = Self {
            seeds,
            transitions: Vec::new(),
        };
        live.trigger(0);
        live
    }

    /// One press of the section shortcut at `at`.
    fn trigger(&mut self, at: Micros) -> SectionId {
        let section = match next_advance(&self.seeds, &self.transitions) {
            Advance::Agenda(id) => id,
            Advance::Additional { number } => {
                let id = SectionId::generate();
                self.seeds.push(SectionSeed {
                    id,
                    origin: SectionOrigin::Additional { number },
                    title: additional_section_title(number),
                });
                id
            }
        };
        self.transitions.push(Transition {
            sequence: self.transitions.len() as u64 + 1,
            at,
            section,
        });
        section
    }

    fn timeline(&self, overrides: &[Override], end: Option<Micros>) -> EffectiveTimeline {
        derive(&self.seeds, &self.transitions, overrides, end).expect("consistent timeline")
    }
}

fn titles(sections: &[EffectiveSection]) -> Vec<&str> {
    sections
        .iter()
        .map(|section| section.title.as_str())
        .collect()
}

fn ranges(sections: &[EffectiveSection]) -> Vec<(Micros, Option<Micros>)> {
    sections
        .iter()
        .map(|section| {
            let range = section.range.expect("timed section");
            (range.start, range.end)
        })
        .collect()
}

fn edit(sequence: u64, edit: Edit) -> Override {
    Override { sequence, edit }
}

// Rule 1: the timeline is a pure function of its inputs.

#[test]
fn deriving_twice_gives_the_same_timeline_whatever_order_the_rows_arrive_in() {
    let mut live = Live::start(agenda(&["Intro", "Budget", "Hiring"]));
    live.trigger(10);
    live.trigger(20);
    let overrides = vec![
        edit(
            1,
            Edit::Rename {
                section: live.seeds[0].id,
                title: "Welcome".into(),
            },
        ),
        edit(
            2,
            Edit::SetBoundary {
                section: live.seeds[1].id,
                at: 12,
            },
        ),
    ];
    let expected = live.timeline(&overrides, Some(30));

    let mut reversed = live.clone();
    reversed.seeds.reverse();
    reversed.transitions.reverse();
    let mut reversed_overrides = overrides.clone();
    reversed_overrides.reverse();

    assert_eq!(reversed.timeline(&reversed_overrides, Some(30)), expected);
}

// Rules 2 and 3: half-open sections; an item on a boundary belongs to the section starting there.

#[test]
fn an_item_on_a_boundary_belongs_to_the_section_that_starts_there() {
    let mut live = Live::start(agenda(&["Intro", "Budget"]));
    live.trigger(1_000);
    let timeline = live.timeline(&[], Some(5_000));

    assert_eq!(
        ranges(timeline.timed()),
        vec![(0, Some(1_000)), (1_000, Some(5_000))]
    );
    let title_at = |at| timeline.section_at(at).expect("a section").title.as_str();
    assert_eq!(title_at(0), "Intro");
    assert_eq!(title_at(999), "Intro");
    assert_eq!(title_at(1_000), "Budget");
    assert_eq!(title_at(4_999), "Budget");
}

// Rule 4: the initial boundary.

#[test]
fn the_first_agenda_item_starts_at_time_zero() {
    let live = Live::start(agenda(&["Intro", "Budget"]));
    let timeline = live.timeline(&[], None);

    assert_eq!(titles(timeline.timed()), vec!["Intro"]);
    assert_eq!(ranges(timeline.timed()), vec![(0, None)]);
}

#[test]
fn with_no_agenda_the_first_section_is_an_additional_section_with_a_default_title() {
    let live = Live::start(Vec::new());
    let timeline = live.timeline(&[], Some(100));

    assert_eq!(titles(timeline.sections()), vec!["Additional section 1"]);
    assert_eq!(
        timeline.sections()[0].origin,
        SectionOrigin::Additional { number: 1 }
    );
    assert_eq!(ranges(timeline.timed()), vec![(0, Some(100))]);
}

#[test]
fn the_initial_section_can_be_renamed_but_not_moved_or_removed() {
    let mut live = Live::start(agenda(&["Intro", "Budget"]));
    live.trigger(50);
    let timeline = live.timeline(&[], Some(100));
    let first = live.seeds[0].id;

    assert_eq!(
        timeline.check_set_boundary(first, 10),
        Err(EditError::InitialBoundary)
    );
    assert_eq!(
        timeline.check_remove_boundary(first),
        Err(EditError::InitialBoundary)
    );

    let renamed = live.timeline(
        &[edit(
            1,
            Edit::Rename {
                section: first,
                title: "Welcome".into(),
            },
        )],
        Some(100),
    );
    assert_eq!(titles(renamed.timed()), vec!["Welcome", "Budget"]);
    assert_eq!(ranges(renamed.timed()), ranges(timeline.timed()));
}

// Rules 5 and 6: the zero and end boundaries.

#[test]
fn no_boundary_can_be_placed_at_or_before_zero_or_at_or_after_the_end() {
    let mut live = Live::start(agenda(&["Intro", "Budget", "Hiring"]));
    live.trigger(50);
    let timeline = live.timeline(&[], Some(100));
    let budget = live.seeds[1].id;
    let unvisited = live.seeds[2].id;

    for at in [-1, 0] {
        assert_eq!(
            timeline.check_insert_section(at),
            Err(EditError::NotAfterStart)
        );
        assert_eq!(
            timeline.check_set_boundary(budget, at),
            Err(EditError::NotAfterStart)
        );
        assert_eq!(
            timeline.check_set_boundary(unvisited, at),
            Err(EditError::NotAfterStart)
        );
    }
    for at in [100, 101] {
        assert_eq!(
            timeline.check_insert_section(at),
            Err(EditError::NotBeforeEnd)
        );
        assert_eq!(
            timeline.check_set_boundary(budget, at),
            Err(EditError::NotBeforeEnd)
        );
        assert_eq!(
            timeline.check_set_boundary(unvisited, at),
            Err(EditError::NotBeforeEnd)
        );
    }
    assert_eq!(timeline.check_insert_section(1), Ok(()));
    assert_eq!(timeline.check_insert_section(99), Ok(()));
}

#[test]
fn an_item_stamped_exactly_at_the_end_belongs_to_the_last_section() {
    let mut live = Live::start(agenda(&["Intro", "Budget"]));
    live.trigger(50);
    let timeline = live.timeline(&[], Some(100));

    assert_eq!(timeline.section_at(100).expect("a section").title, "Budget");
    assert_eq!(timeline.section_at(250).expect("a section").title, "Budget");
}

// Rule 7: ties.

#[test]
fn two_transitions_at_the_same_instant_keep_the_earlier_section_present_and_empty() {
    let mut live = Live::start(agenda(&["Intro", "Budget", "Hiring"]));
    live.trigger(40);
    live.trigger(40);
    let timeline = live.timeline(&[], Some(100));

    assert_eq!(titles(timeline.timed()), vec!["Intro", "Budget", "Hiring"]);
    assert_eq!(
        ranges(timeline.timed()),
        vec![(0, Some(40)), (40, Some(40)), (40, Some(100))]
    );
    // The empty section holds nothing; an item at that instant belongs to the later one.
    assert_eq!(timeline.section_at(40).expect("a section").title, "Hiring");
}

#[test]
fn ties_are_ordered_by_sequence_number_not_by_arrival_order() {
    let seeds = agenda(&["Intro", "Budget", "Hiring"]);
    let transitions = [
        Transition {
            sequence: 3,
            at: 40,
            section: seeds[2].id,
        },
        Transition {
            sequence: 1,
            at: 0,
            section: seeds[0].id,
        },
        Transition {
            sequence: 2,
            at: 40,
            section: seeds[1].id,
        },
    ];
    let timeline = derive(&seeds, &transitions, &[], Some(100)).expect("timeline");

    assert_eq!(titles(timeline.timed()), vec!["Intro", "Budget", "Hiring"]);
}

#[test]
fn a_transition_at_the_very_end_leaves_an_empty_last_section() {
    let mut live = Live::start(agenda(&["Intro", "Budget"]));
    live.trigger(100);
    let timeline = live.timeline(&[], Some(100));

    assert_eq!(
        ranges(timeline.timed()),
        vec![(0, Some(100)), (100, Some(100))]
    );
}

// Rule 8: the shortcut.

#[test]
fn each_trigger_advances_through_the_agenda_then_creates_additional_sections() {
    let mut live = Live::start(agenda(&["Intro", "Budget"]));
    for at in [10, 20, 30, 40] {
        live.trigger(at);
    }
    let timeline = live.timeline(&[], Some(50));

    assert_eq!(
        titles(timeline.timed()),
        vec![
            "Intro",
            "Budget",
            "Additional section 1",
            "Additional section 2",
            "Additional section 3"
        ]
    );
    assert!(timeline.unvisited().is_empty());
}

#[test]
fn additional_sections_after_an_empty_agenda_continue_the_numbering() {
    let mut live = Live::start(Vec::new());
    live.trigger(10);
    live.trigger(20);

    assert_eq!(
        titles(live.timeline(&[], None).timed()),
        vec![
            "Additional section 1",
            "Additional section 2",
            "Additional section 3"
        ]
    );
}

#[test]
fn renaming_an_additional_section_does_not_reuse_its_number() {
    let mut live = Live::start(Vec::new());
    let first = live.transitions[0].section;
    live.seeds[0].title = "Parking lot".into();

    assert_eq!(
        next_advance(&live.seeds, &live.transitions),
        Advance::Additional { number: 2 }
    );
    assert_ne!(live.trigger(10), first);
}

// Rule 9: unvisited agenda items.

#[test]
fn agenda_items_never_reached_stay_in_the_structure_without_a_time_range() {
    let mut live = Live::start(agenda(&["Intro", "Budget", "Hiring", "Any other business"]));
    live.trigger(10);
    let timeline = live.timeline(&[], Some(50));

    assert_eq!(titles(timeline.timed()), vec!["Intro", "Budget"]);
    assert_eq!(
        titles(timeline.unvisited()),
        vec!["Hiring", "Any other business"]
    );
    assert!(
        timeline
            .unvisited()
            .iter()
            .all(|section| section.range.is_none())
    );
    assert_eq!(timeline.sections().len(), 4);
}

// Rule 10: review edits.

#[test]
fn moving_a_boundary_changes_membership_on_both_sides() {
    let mut live = Live::start(agenda(&["Intro", "Budget", "Hiring"]));
    let budget = live.trigger(40);
    live.trigger(80);
    let before = live.timeline(&[], Some(100));
    assert_eq!(before.section_at(30).expect("a section").title, "Intro");
    assert_eq!(before.check_set_boundary(budget, 25), Ok(()));

    let after = live.timeline(
        &[edit(
            1,
            Edit::SetBoundary {
                section: budget,
                at: 25,
            },
        )],
        Some(100),
    );

    assert_eq!(
        ranges(after.timed()),
        vec![(0, Some(25)), (25, Some(80)), (80, Some(100))]
    );
    assert_eq!(after.section_at(30).expect("a section").title, "Budget");
}

#[test]
fn a_boundary_moves_only_between_its_neighbours_and_not_onto_one() {
    let mut live = Live::start(agenda(&["Intro", "Budget", "Hiring", "Close"]));
    let budget = live.trigger(40);
    let hiring = live.trigger(60);
    live.trigger(80);
    let timeline = live.timeline(&[], Some(100));

    assert_eq!(
        timeline.check_set_boundary(hiring, 30),
        Err(EditError::PastNeighbour)
    );
    assert_eq!(
        timeline.check_set_boundary(hiring, 90),
        Err(EditError::PastNeighbour)
    );
    assert_eq!(
        timeline.check_set_boundary(hiring, 40),
        Err(EditError::BoundaryAlreadyThere)
    );
    assert_eq!(
        timeline.check_set_boundary(hiring, 80),
        Err(EditError::BoundaryAlreadyThere)
    );
    assert_eq!(timeline.check_set_boundary(hiring, 41), Ok(()));
    assert_eq!(timeline.check_set_boundary(hiring, 79), Ok(()));
    assert_eq!(timeline.check_set_boundary(budget, 59), Ok(()));
}

#[test]
fn removing_a_boundary_gives_its_time_to_the_section_before() {
    let mut live = Live::start(agenda(&["Intro", "Budget", "Hiring"]));
    let budget = live.trigger(40);
    live.trigger(80);
    let before = live.timeline(&[], Some(100));
    assert_eq!(before.check_remove_boundary(budget), Ok(()));

    let after = live.timeline(
        &[edit(1, Edit::RemoveBoundary { section: budget })],
        Some(100),
    );

    assert_eq!(titles(after.timed()), vec!["Intro", "Hiring"]);
    assert_eq!(ranges(after.timed()), vec![(0, Some(80)), (80, Some(100))]);
    assert_eq!(after.section_at(50).expect("a section").title, "Intro");
    // The agenda item is still part of the meeting, now as one that has no time.
    assert_eq!(titles(after.unvisited()), vec!["Budget"]);
    assert_eq!(
        after.check_remove_boundary(budget),
        Err(EditError::NoBoundary)
    );
}

#[test]
fn removing_the_boundary_of_an_additional_section_takes_it_out_of_the_structure() {
    let mut live = Live::start(agenda(&["Intro"]));
    let extra = live.trigger(40);

    let after = live.timeline(
        &[edit(1, Edit::RemoveBoundary { section: extra })],
        Some(100),
    );

    assert_eq!(titles(after.sections()), vec!["Intro"]);
    assert_eq!(ranges(after.timed()), vec![(0, Some(100))]);
}

#[test]
fn an_unvisited_agenda_item_can_be_given_a_boundary_in_review() {
    let mut live = Live::start(agenda(&["Intro", "Budget", "Hiring"]));
    live.trigger(40);
    let hiring = live.seeds[2].id;
    let before = live.timeline(&[], Some(100));
    assert_eq!(
        before.check_set_boundary(hiring, 40),
        Err(EditError::BoundaryAlreadyThere)
    );
    assert_eq!(before.check_set_boundary(hiring, 70), Ok(()));

    let after = live.timeline(
        &[edit(
            1,
            Edit::SetBoundary {
                section: hiring,
                at: 70,
            },
        )],
        Some(100),
    );

    assert_eq!(titles(after.timed()), vec!["Intro", "Budget", "Hiring"]);
    assert!(after.unvisited().is_empty());
    assert_eq!(after.section_at(70).expect("a section").id, hiring);
}

#[test]
fn inserting_a_section_splits_the_one_it_lands_in() {
    let mut live = Live::start(agenda(&["Intro", "Budget"]));
    live.trigger(40);
    let inserted = SectionId::generate();
    let before = live.timeline(&[], Some(100));
    assert_eq!(
        before.check_insert_section(40),
        Err(EditError::BoundaryAlreadyThere)
    );
    assert_eq!(before.check_insert_section(20), Ok(()));

    let after = live.timeline(
        &[edit(
            1,
            Edit::InsertSection {
                section: inserted,
                at: 20,
                title: "Apologies".into(),
            },
        )],
        Some(100),
    );

    assert_eq!(titles(after.timed()), vec!["Intro", "Apologies", "Budget"]);
    assert_eq!(
        ranges(after.timed()),
        vec![(0, Some(20)), (20, Some(40)), (40, Some(100))]
    );
    assert_eq!(after.timed()[1].origin, SectionOrigin::Inserted);
}

#[test]
fn later_edits_build_on_earlier_ones() {
    let mut live = Live::start(agenda(&["Intro", "Budget"]));
    let budget = live.trigger(40);
    let overrides = [
        edit(1, Edit::RemoveBoundary { section: budget }),
        edit(
            2,
            Edit::SetBoundary {
                section: budget,
                at: 60,
            },
        ),
        edit(
            3,
            Edit::Rename {
                section: budget,
                title: "Budget (revised)".into(),
            },
        ),
    ];

    let timeline = live.timeline(&overrides, Some(100));

    assert_eq!(titles(timeline.timed()), vec!["Intro", "Budget (revised)"]);
    assert_eq!(
        ranges(timeline.timed()),
        vec![(0, Some(60)), (60, Some(100))]
    );
}

#[test]
fn edits_to_sections_that_do_not_exist_are_refused() {
    let live = Live::start(agenda(&["Intro"]));
    let timeline = live.timeline(&[], Some(100));
    let stranger = SectionId::generate();

    assert_eq!(
        timeline.check_set_boundary(stranger, 50),
        Err(EditError::UnknownSection)
    );
    assert_eq!(
        timeline.check_remove_boundary(stranger),
        Err(EditError::UnknownSection)
    );
}

#[test]
fn contradictory_stored_data_is_an_error_not_a_guess() {
    let seeds = agenda(&["Intro", "Budget"]);
    let start = Transition {
        sequence: 1,
        at: 0,
        section: seeds[0].id,
    };
    let stranger = SectionId::generate();

    let twice = [seeds[0].clone(), seeds[0].clone()];
    assert_eq!(
        derive(&twice, &[], &[], None),
        Err(TimelineError::DuplicateSection(seeds[0].id))
    );
    let unknown = Transition {
        sequence: 2,
        at: 5,
        section: stranger,
    };
    assert_eq!(
        derive(&seeds, &[start, unknown], &[], None),
        Err(TimelineError::UnknownSection(stranger))
    );
    let again = Transition {
        sequence: 2,
        at: 5,
        section: seeds[0].id,
    };
    assert_eq!(
        derive(&seeds, &[start, again], &[], None),
        Err(TimelineError::DuplicateTransition(seeds[0].id))
    );
    let remove_unvisited = edit(
        1,
        Edit::RemoveBoundary {
            section: seeds[1].id,
        },
    );
    assert_eq!(
        derive(&seeds, &[start], &[remove_unvisited], None),
        Err(TimelineError::NoBoundary(seeds[1].id))
    );
    let insert_existing = edit(
        1,
        Edit::InsertSection {
            section: seeds[1].id,
            at: 5,
            title: "Again".into(),
        },
    );
    assert_eq!(
        derive(&seeds, &[start], &[insert_existing], None),
        Err(TimelineError::DuplicateSection(seeds[1].id))
    );
}

#[test]
fn before_recording_there_are_only_unvisited_agenda_items() {
    let seeds = agenda(&["Intro", "Budget"]);
    let timeline = derive(&seeds, &[], &[], None).expect("timeline");

    assert!(timeline.timed().is_empty());
    assert_eq!(titles(timeline.unvisited()), vec!["Intro", "Budget"]);
    assert!(timeline.section_at(0).is_none());
}

// Property tests: the rules hold for any recording and any sequence of accepted edits.

#[derive(Debug, Clone)]
struct Scenario {
    live: Live,
    end: Micros,
}

fn scenario() -> impl Strategy<Value = Scenario> {
    (
        0usize..6,
        proptest::collection::vec(0i64..4, 0..12),
        0i64..5,
    )
        .prop_map(|(agenda_len, gaps, tail)| {
            let names: Vec<String> = (0..agenda_len)
                .map(|index| format!("Item {index}"))
                .collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let mut live = Live::start(agenda(&names));
            let mut at = 0;
            for gap in gaps {
                // A gap of zero is a tie with the transition before it.
                at += gap;
                live.trigger(at);
            }
            Scenario {
                live,
                end: at + tail,
            }
        })
}

/// A review edit the user might attempt. Sections are picked by index into the current
/// display order, so every attempt refers to something on screen.
#[derive(Debug, Clone)]
enum Attempt {
    Rename(usize),
    SetBoundary(usize, Micros),
    Remove(usize),
    Insert(Micros),
}

fn attempts() -> impl Strategy<Value = Vec<Attempt>> {
    let time = -1i64..48;
    let attempt = prop_oneof![
        (0usize..16).prop_map(Attempt::Rename),
        ((0usize..16), time.clone()).prop_map(|(pick, at)| Attempt::SetBoundary(pick, at)),
        (0usize..16).prop_map(Attempt::Remove),
        time.prop_map(Attempt::Insert),
    ];
    proptest::collection::vec(attempt, 0..12)
}

/// Applies each attempt the timeline accepts, as the app would, and returns the saved edits.
fn accepted(scenario: &Scenario, attempts: &[Attempt]) -> Vec<Override> {
    let mut overrides: Vec<Override> = Vec::new();
    for attempt in attempts {
        let current = scenario.live.timeline(&overrides, Some(scenario.end));
        let pick = |index: usize| current.sections()[index % current.sections().len()].id;
        let sequence = overrides.len() as u64 + 1;
        let allowed = match attempt {
            Attempt::Rename(index) => Some(Edit::Rename {
                section: pick(*index),
                title: format!("Renamed {sequence}"),
            }),
            Attempt::SetBoundary(index, at) => {
                let section = pick(*index);
                current
                    .check_set_boundary(section, *at)
                    .is_ok()
                    .then_some(Edit::SetBoundary { section, at: *at })
            }
            Attempt::Remove(index) => {
                let section = pick(*index);
                current
                    .check_remove_boundary(section)
                    .is_ok()
                    .then_some(Edit::RemoveBoundary { section })
            }
            Attempt::Insert(at) => {
                current
                    .check_insert_section(*at)
                    .is_ok()
                    .then(|| Edit::InsertSection {
                        section: SectionId::generate(),
                        at: *at,
                        title: format!("Inserted {sequence}"),
                    })
            }
        };
        if let Some(edit) = allowed {
            overrides.push(Override { sequence, edit });
        }
    }
    overrides
}

/// The structural rules every effective timeline of a stopped recording obeys.
fn assert_well_formed(timeline: &EffectiveTimeline, scenario: &Scenario) {
    let end = scenario.end;
    let timed = timeline.timed();

    // Rule 4: the section that started the recording is still first and still starts at zero.
    let first = timed
        .first()
        .expect("a recording always has a first section");
    assert_eq!(first.id, scenario.live.transitions[0].section);
    assert_eq!(first.range.expect("timed").start, 0);

    // Rule 2: sections are contiguous half-open intervals that cover the whole recording.
    for pair in timed.windows(2) {
        let (left, right) = (pair[0].range.expect("timed"), pair[1].range.expect("timed"));
        assert_eq!(left.end, Some(right.start));
        assert!(left.start <= right.start);
    }
    let last = timed.last().expect("a last section").range.expect("timed");
    assert_eq!(last.end, Some(end.max(last.start)));
    for section in timed {
        let range = section.range.expect("timed");
        assert!((0..=end).contains(&range.start), "{range:?} in 0..={end}");
    }

    // Rule 9: every agenda item is in the structure exactly once, visited or not.
    let mut seen = HashSet::new();
    for section in timeline.sections() {
        assert!(seen.insert(section.id), "section listed twice");
    }
    for seed in &scenario.live.seeds {
        if matches!(seed.origin, SectionOrigin::Agenda { .. }) {
            assert!(seen.contains(&seed.id), "agenda item dropped");
        }
    }
    let positions: Vec<u32> = timeline
        .unvisited()
        .iter()
        .map(|section| {
            assert_eq!(section.range, None);
            match section.origin {
                SectionOrigin::Agenda { position } => position,
                other => panic!("only agenda items can be unvisited, found {other:?}"),
            }
        })
        .collect();
    assert!(positions.is_sorted());

    // Rules 2, 3, 6 and 7: every instant belongs to exactly one section.
    for at in -1..=end + 1 {
        let owner = timeline.section_at(at).expect("an owner");
        let range = owner.range.expect("owner is timed");
        if (0..end).contains(&at) {
            assert!(range.start <= at && range.end.is_some_and(|range_end| at < range_end));
            let containing = timed
                .iter()
                .filter(|section| {
                    let range = section.range.expect("timed");
                    range.start <= at && range.end.is_some_and(|range_end| at < range_end)
                })
                .count();
            assert_eq!(containing, 1, "instant {at} is in {containing} sections");
        } else if at >= end {
            assert_eq!(owner.id, timed.last().expect("a last section").id);
        } else {
            assert_eq!(owner.id, first.id);
        }
    }
}

proptest! {
    // A failure prints the shrunk case; nothing is written into the source tree.
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    // Rules 7, 8 and 9: a recording drops nothing, however the shortcut was pressed.
    #[test]
    fn every_trigger_starts_a_distinct_section_and_none_wraps(scenario in scenario()) {
        let timeline = scenario.live.timeline(&[], Some(scenario.end));
        assert_well_formed(&timeline, &scenario);

        prop_assert_eq!(timeline.timed().len(), scenario.live.transitions.len());
        // Agenda items are started in agenda order, then additional sections in number order.
        let mut next_position = 0;
        let mut next_number = 1;
        for section in timeline.timed() {
            match section.origin {
                SectionOrigin::Agenda { position } => {
                    prop_assert_eq!(next_number, 1, "agenda item started after an additional one");
                    prop_assert_eq!(position, next_position);
                    next_position += 1;
                }
                SectionOrigin::Additional { number } => {
                    prop_assert_eq!(number, next_number);
                    next_number += 1;
                }
                SectionOrigin::Inserted => prop_assert!(false, "nothing is inserted while recording"),
            }
        }
        let agenda_len = scenario
            .live
            .seeds
            .iter()
            .filter(|seed| matches!(seed.origin, SectionOrigin::Agenda { .. }))
            .count();
        prop_assert_eq!(next_position as usize + timeline.unvisited().len(), agenda_len);
        // Additional sections appear only once the agenda is exhausted.
        prop_assert!(next_number == 1 || timeline.unvisited().is_empty());
    }

    // Rules 4, 5, 6 and 10: accepted edits keep the timeline well formed.
    #[test]
    fn any_sequence_of_accepted_edits_keeps_the_timeline_well_formed(
        scenario in scenario(),
        attempts in attempts(),
    ) {
        let overrides = accepted(&scenario, &attempts);
        let timeline = scenario.live.timeline(&overrides, Some(scenario.end));
        assert_well_formed(&timeline, &scenario);
    }

    // Rule 1: the same rows in any order give the same timeline.
    #[test]
    fn the_timeline_does_not_depend_on_row_order(
        scenario in scenario(),
        attempts in attempts(),
    ) {
        let overrides = accepted(&scenario, &attempts);
        let expected = scenario.live.timeline(&overrides, Some(scenario.end));

        let mut shuffled = scenario.live.clone();
        shuffled.transitions.reverse();
        let mut reversed = overrides.clone();
        reversed.reverse();

        prop_assert_eq!(shuffled.timeline(&reversed, Some(scenario.end)), expected);
    }

    // Rule 10: a rename changes the title and nothing else.
    #[test]
    fn renaming_changes_no_boundary_and_no_membership(
        scenario in scenario(),
        attempts in attempts(),
        pick in 0usize..16,
    ) {
        let mut overrides = accepted(&scenario, &attempts);
        let before = scenario.live.timeline(&overrides, Some(scenario.end));
        let target = before.sections()[pick % before.sections().len()].id;
        overrides.push(Override {
            sequence: overrides.len() as u64 + 1,
            edit: Edit::Rename { section: target, title: "A new name".into() },
        });

        let after = scenario.live.timeline(&overrides, Some(scenario.end));

        prop_assert_eq!(before.sections().len(), after.sections().len());
        for (old, new) in before.sections().iter().zip(after.sections()) {
            prop_assert_eq!(old.id, new.id);
            prop_assert_eq!(old.range, new.range);
            prop_assert_eq!(old.origin, new.origin);
            if old.id == target {
                prop_assert_eq!(new.title.as_str(), "A new name");
            } else {
                prop_assert_eq!(&old.title, &new.title);
            }
        }
        for at in 0..=scenario.end {
            prop_assert_eq!(
                before.section_at(at).map(|section| section.id),
                after.section_at(at).map(|section| section.id)
            );
        }
    }

    // Rule 10: an accepted boundary puts the section exactly where it was asked to start.
    #[test]
    fn a_placed_boundary_owns_the_instant_it_was_placed_at(
        scenario in scenario(),
        attempts in attempts(),
        pick in 0usize..16,
        at in 0i64..48,
    ) {
        let mut overrides = accepted(&scenario, &attempts);
        let before = scenario.live.timeline(&overrides, Some(scenario.end));
        let target = before.sections()[pick % before.sections().len()].id;
        let at = at % (scenario.end + 1);
        // Most picks are refused; the property is about the ones that are accepted.
        if before.check_set_boundary(target, at).is_ok() {
            overrides.push(Override {
                sequence: overrides.len() as u64 + 1,
                edit: Edit::SetBoundary { section: target, at },
            });

            let after = scenario.live.timeline(&overrides, Some(scenario.end));

            prop_assert_eq!(after.section_at(at).map(|section| section.id), Some(target));
            assert_well_formed(&after, &scenario);
        }
    }
}
